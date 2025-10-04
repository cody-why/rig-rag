use axum::{
    Router,
    extract::{Json, Multipart, Path, Query, State},
    http::StatusCode,
    response::Json as ResponseJson,
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info};

use crate::{
    agent::RigAgent,
    db::{Document, DocumentStore},
};

use crate::web::ChatStore;

#[derive(Debug, Deserialize)]
pub struct CreateDocumentRequest {
    pub filename: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDocumentRequest {
    pub filename: Option<String>,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct DocumentResponse {
    pub id: String,
    pub filename: String,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct DocumentListResponse {
    pub documents: Vec<DocumentListItem>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}
#[derive(Debug, Serialize)]
pub struct DocumentListItem {
    pub id: String,
    pub filename: String,
    pub preview: String,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Deserialize, Default)]
struct PaginationQuery {
    limit: Option<usize>,
    offset: Option<usize>,
}

impl From<Document> for DocumentResponse {
    fn from(doc: Document) -> Self {
        DocumentResponse {
            id: doc.id,
            filename: doc.source, // 使用 source 作为 filename
            content: doc.content,
            created_at: doc.created_at.to_rfc3339(),
            updated_at: doc.updated_at.to_rfc3339(),
        }
    }
}

pub fn create_document_router() -> Router<(Arc<RigAgent>, Option<Arc<DocumentStore>>, ChatStore)> {
    Router::new()
        .route("/api/documents", get(list_documents))
        .route("/api/documents", post(create_document))
        .route("/api/documents/upload", post(upload_document))
        .route("/api/documents/reset", post(reset_documents))
        .route("/api/documents/{id}", get(get_document))
        .route("/api/documents/{id}", put(update_document))
        .route("/api/documents/{id}", delete(delete_document))
}

async fn list_documents(
    State((_, document_store, _)): State<(Arc<RigAgent>, Option<Arc<DocumentStore>>, ChatStore)>,
    Query(p): Query<PaginationQuery>,
) -> Result<ResponseJson<DocumentListResponse>, StatusCode> {
    match document_store {
        Some(store) => {
            let limit = p.limit.unwrap_or(20).clamp(1, 1000);
            let offset = p.offset.unwrap_or(0);
            match store.list_documents_paginated(limit, offset).await {
                Ok((docs, total)) => {
                    let documents = docs
                        .into_iter()
                        .map(|doc| {
                            let mut preview: String = doc.content.chars().take(160).collect();
                            if preview.len() < doc.content.len() {
                                preview.push_str("...");
                            }
                            DocumentListItem {
                                id: doc.id,
                                filename: doc.source,
                                preview,
                                created_at: doc.created_at.to_rfc3339(),
                                updated_at: doc.updated_at.to_rfc3339(),
                            }
                        })
                        .collect();
                    Ok(ResponseJson(DocumentListResponse {
                        documents,
                        total,
                        limit,
                        offset,
                    }))
                }
                Err(e) => {
                    error!("Failed to list documents: {}", e);
                    Err(StatusCode::INTERNAL_SERVER_ERROR)
                }
            }
        }
        None => {
            error!("Document store not available");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

async fn get_document(
    State((_, document_store, _)): State<(Arc<RigAgent>, Option<Arc<DocumentStore>>, ChatStore)>,
    Path(id): Path<String>,
) -> Result<ResponseJson<DocumentResponse>, StatusCode> {
    match document_store {
        Some(store) => match store.get_document(&id).await {
            Ok(Some(doc)) => Ok(ResponseJson(DocumentResponse::from(doc))),
            Ok(None) => Err(StatusCode::NOT_FOUND),
            Err(e) => {
                error!("Failed to get document: {}", e);
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        },
        None => {
            error!("Document store not available");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

async fn create_document(
    State((agent, document_store, _)): State<(
        Arc<RigAgent>,
        Option<Arc<DocumentStore>>,
        ChatStore,
    )>,
    Json(req): Json<CreateDocumentRequest>,
) -> Result<ResponseJson<DocumentResponse>, StatusCode> {
    info!("Creating document");
    match document_store {
        Some(store) => {
            let doc = Document::new(req.content, req.filename);

            // 获取 embedding model 从 agent context
            let embedding_model = {
                let context = agent.context.read().unwrap();
                context.embedding_model.clone()
            };

            match store
                .add_documents_with_embeddings(vec![doc.clone()], embedding_model)
                .await
            {
                Ok(_) => {
                    info!("Created document: {}", doc.id);

                    // 标记agent需要重建以使用新文档
                    if let Ok(mut context) = agent.context.write() {
                        context.needs_rebuild = true;
                        info!("Marked agent for rebuild due to new document");
                    }

                    Ok(ResponseJson(DocumentResponse::from(doc)))
                }
                Err(e) => {
                    error!("Failed to create document: {}", e);
                    Err(StatusCode::INTERNAL_SERVER_ERROR)
                }
            }
        }
        None => {
            error!("Document store not available");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

async fn update_document(
    State((agent, document_store, _)): State<(
        Arc<RigAgent>,
        Option<Arc<DocumentStore>>,
        ChatStore,
    )>,
    Path(id): Path<String>,
    Json(req): Json<UpdateDocumentRequest>,
) -> Result<ResponseJson<DocumentResponse>, StatusCode> {
    info!("Updating document");
    match document_store {
        Some(store) => match store.get_document(&id).await {
            Ok(Some(mut doc)) => {
                doc.content = req.content;
                if let Some(filename) = req.filename {
                    doc.source = filename;
                }
                doc.updated_at = chrono::Utc::now();

                // 删除旧文档并添加新文档
                if let Err(e) = store.delete_document(&id).await {
                    error!("Failed to delete old document: {}", e);
                }

                // 获取 embedding model 从 agent context
                let embedding_model = {
                    let context = agent.context.read().unwrap();
                    context.embedding_model.clone()
                };

                match store
                    .add_documents_with_embeddings(vec![doc.clone()], embedding_model)
                    .await
                {
                    Ok(_) => {
                        info!("Updated document: {}", doc.id);

                        // 标记agent需要重建以使用更新的文档
                        if let Ok(mut context) = agent.context.write() {
                            context.needs_rebuild = true;
                            info!("Marked agent for rebuild due to updated document");
                        }

                        Ok(ResponseJson(DocumentResponse::from(doc)))
                    }
                    Err(e) => {
                        error!("Failed to update document: {}", e);
                        Err(StatusCode::INTERNAL_SERVER_ERROR)
                    }
                }
            }
            Ok(None) => Err(StatusCode::NOT_FOUND),
            Err(e) => {
                error!("Failed to get document: {}", e);
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        },
        None => {
            error!("Document store not available");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

async fn delete_document(
    State((_agent, document_store, _)): State<(
        Arc<RigAgent>,
        Option<Arc<DocumentStore>>,
        ChatStore,
    )>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    info!("Deleting document");
    match document_store {
        Some(store) => match store.delete_document(&id).await {
            Ok(_) => {
                info!("Deleted document: {}", id);

                // LanceDB 是持久化的，不需要同步向量存储

                Ok(StatusCode::NO_CONTENT)
            }
            Err(e) => {
                error!("Failed to delete document: {}", e);
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        },
        None => {
            error!("Document store not available");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

async fn upload_document(
    State((agent, document_store, _)): State<(
        Arc<RigAgent>,
        Option<Arc<DocumentStore>>,
        ChatStore,
    )>,
    mut multipart: Multipart,
) -> Result<ResponseJson<DocumentResponse>, StatusCode> {
    info!("Uploading document");
    match document_store {
        Some(store) => {
            let mut filename = String::new();
            let mut content = String::new();

            while let Some(field) = multipart
                .next_field()
                .await
                .map_err(|_| StatusCode::BAD_REQUEST)?
            {
                let name = field.name().unwrap_or_default().to_string();
                let data = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;

                match name.as_str() {
                    "filename" => {
                        filename = String::from_utf8(data.to_vec())
                            .map_err(|_| StatusCode::BAD_REQUEST)?;
                    }
                    "file" => {
                        content = String::from_utf8(data.to_vec())
                            .map_err(|_| StatusCode::BAD_REQUEST)?;
                    }
                    _ => {}
                }
            }

            if filename.is_empty() || content.is_empty() {
                return Err(StatusCode::BAD_REQUEST);
            }

            let doc = Document::new(content, filename);

            // 获取 embedding model 从 agent context
            let embedding_model = {
                let context = agent.context.read().unwrap();
                context.embedding_model.clone()
            };

            match store
                .add_documents_with_embeddings(vec![doc.clone()], embedding_model)
                .await
            {
                Ok(_) => {
                    info!("Uploaded document: {}", doc.id);

                    // 标记agent需要重建以使用新上传的文档
                    if let Ok(mut context) = agent.context.write() {
                        context.needs_rebuild = true;
                        info!("Marked agent for rebuild due to uploaded document");
                    }

                    Ok(ResponseJson(DocumentResponse::from(doc)))
                }
                Err(e) => {
                    error!("Failed to upload document: {}", e);
                    Err(StatusCode::INTERNAL_SERVER_ERROR)
                }
            }
        }
        None => {
            error!("Document store not available");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

async fn reset_documents(
    State((agent, document_store, _)): State<(
        Arc<RigAgent>,
        Option<Arc<DocumentStore>>,
        ChatStore,
    )>,
) -> Result<StatusCode, StatusCode> {
    info!("Resetting document store");
    match document_store {
        Some(store) => match store.reset_table().await {
            Ok(_) => {
                info!("Successfully reset document store");

                // 标记agent需要重建
                if let Ok(mut context) = agent.context.write() {
                    context.needs_rebuild = true;
                    info!("Marked agent for rebuild due to document store reset");
                }

                Ok(StatusCode::OK)
            }
            Err(e) => {
                error!("Failed to reset document store: {}", e);
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        },
        None => {
            error!("Document store not available");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}
