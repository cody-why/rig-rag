use std::sync::Arc;

use axum::{Router, extract::{Json, Multipart, Path, Query, State}, http::StatusCode, response::{IntoResponse, Json as ResponseJson, Response}, routing::{delete, get, post, put}};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::utils::DocumentParser;
use crate::{agent::RigAgent, db::{Document, DocumentStore}};

// State 类型别名
pub type AppState = (Arc<RigAgent>, Arc<DocumentStore>);

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
pub struct ErrorResponse {
    pub error: String,
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

/// 创建文档路由 - 查询操作（所有登录用户可访问）
pub fn create_document_query_router() -> Router<AppState> {
    Router::new()
        .route("/api/documents", get(list_documents))
        .route("/api/documents/{id}", get(get_document))
}

/// 创建文档路由 - 修改操作（仅管理员可访问）
pub fn create_document_mutation_router() -> Router<AppState> {
    Router::new()
        .route("/api/documents", post(create_document))
        .route("/api/documents/upload", post(upload_document))
        // .route("/api/documents/reset", post(reset_documents))
        .route("/api/documents/{id}", put(update_document))
        .route("/api/documents/{id}", delete(delete_document))
}

async fn list_documents(
    State((_, document_store)): State<AppState>, Query(p): Query<PaginationQuery>,
) -> Result<ResponseJson<DocumentListResponse>, StatusCode> {
    let limit = p.limit.unwrap_or(20).clamp(1, 1000);
    let offset = p.offset.unwrap_or(0);
    match document_store.list_documents_paginated(limit, offset).await {
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
        },
        Err(e) => {
            error!("Failed to list documents: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        },
    }
}

async fn get_document(
    State((_, document_store)): State<AppState>, Path(id): Path<String>,
) -> Result<ResponseJson<DocumentResponse>, StatusCode> {
    match document_store.get_document(&id).await {
        Ok(Some(doc)) => Ok(ResponseJson(DocumentResponse::from(doc))),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(e) => {
            error!("Failed to get document: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        },
    }
}

async fn create_document(
    State((agent, document_store)): State<AppState>, Json(req): Json<CreateDocumentRequest>,
) -> Response {
    info!("Creating document");

    process_and_save_document(
        agent,
        document_store,
        &req.filename,
        &req.content,
        "Created",
    )
    .await
    .map_err(|(status, error)| (status, ResponseJson(ErrorResponse { error })))
    .into_response()
}

async fn update_document(
    State((agent, document_store)): State<AppState>, Path(id): Path<String>,
    Json(req): Json<UpdateDocumentRequest>,
) -> Result<ResponseJson<DocumentResponse>, StatusCode> {
    info!("Updating document");
    match document_store.get_document(&id).await {
        Ok(Some(mut doc)) => {
            doc.content = req.content.clone();
            if let Some(filename) = req.filename.clone() {
                doc.source = filename;
            }
            doc.updated_at = chrono::Utc::now();

            // 删除旧文档并添加新文档
            if let Err(e) = document_store.delete_document(&id).await {
                error!("Failed to delete old document: {}", e);
            }

            // 获取 embedding model 从 agent context
            let embedding_model = {
                let context = agent.context.read();
                context.embedding_model.clone()
            };

            match document_store
                .add_documents_with_embeddings(vec![doc.clone()], embedding_model)
                .await
            {
                Ok(_) => {
                    info!("Updated document: {}", doc.id);

                    // 保存文件备份
                    if let Some(backup) = crate::utils::get_file_backup() {
                        match backup.save_backup(&doc.id, &doc.source, &doc.content).await {
                            Ok(path) => {
                                info!("💾 Updated backup to: {:?}", path);
                            },
                            Err(e) => {
                                warn!("⚠️ Failed to save updated backup: {}", e);
                            },
                        }
                    }

                    // 标记agent需要重建以使用更新的文档
                    agent.set_needs_rebuild(true).await;
                    info!("Marked agent for rebuild due to updated document");

                    Ok(ResponseJson(DocumentResponse::from(doc)))
                },
                Err(e) => {
                    error!("Failed to update document: {}", e);
                    Err(StatusCode::INTERNAL_SERVER_ERROR)
                },
            }
        },
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(e) => {
            error!("Failed to get document: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        },
    }
}

async fn delete_document(
    State((agent, document_store)): State<AppState>, Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    info!("Deleting document: {}", id);
    // 首先检查这个文档是否存在，以及是否是分块文档
    match document_store.get_document(&id).await {
        Ok(Some(doc)) => {
            // 检查是否是分块文档（通过source字段判断）
            let is_chunked = doc.source.contains(" (Part ");

            let (delete_id, backup_id) = if is_chunked {
                // 分块文档：提取base_id，删除所有分块
                let parts: Vec<&str> = id.split('-').collect();
                let base_id = parts[..parts.len() - 1].join("-");
                (format!("{}_CHUNKED", base_id), base_id)
            } else {
                // 单文档：直接使用原ID（现在单文档不再有-0后缀）
                (id.clone(), id.clone())
            };

            // 删除文档
            match document_store.delete_document(&delete_id).await {
                Ok(_) => {
                    info!("Deleted document(s) with base ID: {}", backup_id);

                    // 删除文件备份
                    if let Some(backup) = crate::utils::get_file_backup() {
                        match backup.delete_backup(&backup_id).await {
                            Ok(count) => {
                                info!("🗑️  Deleted {} backup file(s) for ID: {}", count, backup_id);
                            },
                            Err(e) => {
                                warn!("⚠️ Failed to delete backup for ID {}: {}", backup_id, e);
                            },
                        }
                    }

                    // 🔧 标记agent需要重建以排除已删除的文档
                    agent.set_needs_rebuild(true).await;
                    info!("Marked agent for rebuild due to document deletion");

                    Ok(StatusCode::NO_CONTENT)
                },
                Err(e) => {
                    error!("Failed to delete document: {}", e);
                    Err(StatusCode::INTERNAL_SERVER_ERROR)
                },
            }
        },
        Ok(None) => {
            error!("Document not found: {}", id);
            Err(StatusCode::NOT_FOUND)
        },
        Err(e) => {
            error!("Failed to get document: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        },
    }
}

async fn upload_document(
    State((agent, document_store)): State<AppState>, mut multipart: Multipart,
) -> Response {
    info!("Uploading document");
    let mut filename = String::new();
    let mut file_data = None;

    // 读取multipart字段
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let name = field.name().unwrap_or_default().to_string();
                let data = match field.bytes().await {
                    Ok(d) => d,
                    Err(e) => {
                        error!("Failed to read field data: {}", e);
                        return (
                            StatusCode::BAD_REQUEST,
                            ResponseJson(ErrorResponse {
                                error: "读取文件数据失败".to_string(),
                            }),
                        )
                            .into_response();
                    },
                };

                match name.as_str() {
                    "filename" => {
                        filename = match String::from_utf8(data.to_vec()) {
                            Ok(s) => s,
                            Err(e) => {
                                error!("Invalid filename encoding: {}", e);
                                return (
                                    StatusCode::BAD_REQUEST,
                                    ResponseJson(ErrorResponse {
                                        error: "文件名编码无效".to_string(),
                                    }),
                                )
                                    .into_response();
                            },
                        };
                    },
                    "file" => {
                        file_data = Some(data);
                    },
                    _ => {},
                }
            },
            Ok(None) => break,
            Err(e) => {
                error!("Failed to read multipart field: {}", e);
                return (
                    StatusCode::BAD_REQUEST,
                    ResponseJson(ErrorResponse {
                        error: "无效的上传请求".to_string(),
                    }),
                )
                    .into_response();
            },
        }
    }

    if filename.is_empty() || file_data.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            ResponseJson(ErrorResponse {
                error: "缺少文件名或文件内容".to_string(),
            }),
        )
            .into_response();
    }

    let file_data = file_data.unwrap();

    // 解析文档内容
    let content = match DocumentParser::parse(&filename, file_data).await {
        Ok(text) => text,
        Err(e) => {
            error!("Failed to parse document {}: {}", filename, e);

            if e.to_string().contains("Unsupported file type") {
                let supported = DocumentParser::supported_extensions().join(", ");
                return (
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    ResponseJson(ErrorResponse {
                        error: format!("不支持的文件类型。支持的格式：{}", supported),
                    }),
                )
                    .into_response();
            }

            return (
                StatusCode::BAD_REQUEST,
                ResponseJson(ErrorResponse {
                    error: format!("文档解析失败: {}", e),
                }),
            )
                .into_response();
        },
    };

    info!(
        "Parsed document '{}', extracted {} chars",
        filename,
        content.len()
    );

    // 处理文档
    match process_and_save_document(agent, document_store, &filename, &content, "Uploaded").await {
        Ok(response) => response.into_response(),
        Err(status) => {
            error!("Failed to upload document: {}", status.1);
            (status.0, ResponseJson(ErrorResponse { error: status.1 })).into_response()
        },
    }
}

#[allow(dead_code)]
async fn reset_documents(
    State((agent, document_store)): State<AppState>,
) -> Result<StatusCode, StatusCode> {
    info!("Resetting document store");
    match document_store.reset_table().await {
        Ok(_) => {
            info!("Successfully reset document store");

            // 标记agent需要重建
            agent.set_needs_rebuild(true).await;
            info!("Marked agent for rebuild due to document store reset");

            Ok(StatusCode::OK)
        },
        Err(e) => {
            error!("Failed to reset document store: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        },
    }
}

/// 处理并保存文档（包含分块、embedding、备份）
async fn process_and_save_document(
    agent: Arc<RigAgent>,
    document_store: Arc<DocumentStore>,
    filename: &str,
    content: &str,
    action: &str, // "Created" 或 "Uploaded"
) -> Result<ResponseJson<DocumentResponse>, (StatusCode, String)> {
    // 检查文件是否为空
    if content.trim().is_empty() {
        warn!("⚠️ Attempted to upload empty file: {}", filename);
        return Err((StatusCode::BAD_REQUEST, "文件内容不能为空".to_string()));
    }

    // 将文档内容分块处理，避免超过embedding模型的token限制
    const CHUNK_SIZE: usize = 12000;
    let chunks = chunk_document(content, CHUNK_SIZE);
    let total_chunks = chunks.len();

    // 双重检查：确保chunks不为空
    if total_chunks == 0 {
        error!(
            "Document '{}' resulted in 0 chunks after processing",
            filename
        );
        return Err((StatusCode::BAD_REQUEST, "文件内容不能为空".to_string()));
    }

    info!("Split document '{}' into {} chunks", filename, total_chunks);

    // 为每个块创建一个Document
    let base_id = nanoid::nanoid!();
    let documents: Vec<Document> = chunks
        .into_iter()
        .enumerate()
        .map(|(idx, chunk_content)| {
            let source = if total_chunks > 1 {
                format!("{} (Part {}/{})", filename, idx + 1, total_chunks)
            } else {
                filename.to_string()
            };
            let id = if total_chunks == 1 {
                base_id.clone()
            } else {
                format!("{}-{}", base_id, idx)
            };
            let timestamp = chrono::Utc::now();
            Document {
                id,
                content: chunk_content,
                source,
                created_at: timestamp,
                updated_at: timestamp,
            }
        })
        .collect();

    // 获取 embedding model 从 agent context
    let embedding_model = {
        let context = agent.context.read();
        context.embedding_model.clone()
    };

    match document_store
        .add_documents_with_embeddings(documents.clone(), embedding_model)
        .await
    {
        Ok(_) => {
            info!(
                "{} document '{}' as {} chunks with base ID: {}",
                action,
                filename,
                documents.len(),
                base_id
            );

            // 保存文件备份
            if let Some(backup) = crate::utils::get_file_backup() {
                match backup.save_backup(&base_id, filename, content).await {
                    Ok(path) => {
                        info!("💾 Saved backup to: {:?}", path);
                    },
                    Err(e) => {
                        warn!("⚠️ Failed to save backup: {}", e);
                    },
                }
            }

            // 标记agent需要重建以使用新文档
            agent.set_needs_rebuild(true).await;
            info!(
                "Marked agent for rebuild due to {} document",
                action.to_lowercase()
            );

            Ok(ResponseJson(DocumentResponse::from(documents[0].clone())))
        },
        Err(e) => {
            error!("Failed to {} document: {}", action.to_lowercase(), e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "保存文档失败".to_string(),
            ))
        },
    }
}

/// 智能分块文本，尝试在句子边界处分割，保持表格完整性
///
/// 这个函数将大文档分成小块，避免超过embedding模型的token限制
/// 特别处理：识别并保持 Markdown 表格的完整性，不在表格中间截断
fn chunk_document(text: &str, chunk_size: usize) -> Vec<String> {
    // 预分配合理容量
    let estimated_chunks = (text.len() / chunk_size).max(1);
    let mut chunks = Vec::with_capacity(estimated_chunks);
    let mut current_chunk = String::with_capacity(chunk_size);
    let mut current_size = 0;

    // 首先将文本分成段落
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        // 跳过开头的空行
        if lines[i].trim().is_empty() {
            i += 1;
            continue;
        }

        // 检测是否是表格的开始（连续两行包含 |）
        if is_table_start(&lines, i) {
            // 检查前面是否有标题（最近的非空行是否是 Markdown 标题）
            let mut title_line: Option<String> = None;
            let mut title_size = 0;

            // 向前查找最近的非空行
            for j in (0..i).rev() {
                let line = lines[j].trim();
                if !line.is_empty() {
                    // 检查是否是 Markdown 标题
                    if line.starts_with('#') {
                        title_line = Some(format!("{}\n\n", line));
                        title_size = title_line.as_ref().unwrap().len();

                        // 如果当前块已经包含了这个标题，不重复添加
                        if !current_chunk.contains(line) {
                            // 需要包含标题
                            if current_size > 0 && !current_chunk.trim().is_empty() {
                                // 当前块有内容，需要先保存
                                chunks.push(current_chunk.trim().to_string());
                                current_chunk = String::new();
                                current_size = 0;
                            }
                        } else {
                            // 标题已在当前块中
                            title_line = None;
                            title_size = 0;
                        }
                    }
                    break;
                }
            }

            // 收集整个表格
            let (table_text, table_end) = collect_table(&lines, i);
            let table_with_newlines = format!("{}\n\n", table_text);
            let total_size = title_size + table_with_newlines.len();

            // 如果当前块加上标题+表格会超出大小，先保存当前块
            if current_size + total_size > chunk_size && current_size > 0 {
                if !current_chunk.trim().is_empty() {
                    chunks.push(current_chunk.trim().to_string());
                }
                current_chunk = String::new();
                current_size = 0;
            }

            // 如果表格本身太大，需要分割表格
            if total_size > chunk_size {
                if !current_chunk.trim().is_empty() {
                    chunks.push(current_chunk.trim().to_string());
                    current_chunk = String::new();
                    current_size = 0;
                }

                // 分割大表格，每个块都带标题
                let table_chunks = split_large_table(&table_text, chunk_size);

                // 如果有标题，将标题添加到每个块的开头
                if let Some(ref title) = title_line {
                    for table_chunk in table_chunks {
                        chunks.push(format!("{}{}", title, table_chunk));
                    }
                } else {
                    chunks.extend(table_chunks);
                }
            } else {
                // 添加标题（如果有）
                if let Some(title) = title_line {
                    current_chunk.push_str(&title);
                    current_size += title_size;
                }

                current_chunk.push_str(&table_with_newlines);
                current_size += table_with_newlines.len();
            }

            i = table_end + 1;
        } else {
            // 普通行，收集段落（空行分隔）
            let current_line = lines[i];

            // 如果是标题，检查下一个非空行是否是表格
            if current_line.trim().starts_with('#') {
                // 检查后面是否有表格
                let mut has_table_after = false;
                for j in (i + 1)..lines.len() {
                    let line = lines[j].trim();
                    if line.is_empty() {
                        continue;
                    }
                    if is_table_start(&lines, j) {
                        has_table_after = true;
                    }
                    break;
                }

                // 如果后面有表格，先跳过这个标题，让表格处理逻辑来处理
                if has_table_after {
                    i += 1;
                    // 跳过空行
                    while i < lines.len() && lines[i].trim().is_empty() {
                        i += 1;
                    }
                    continue;
                }
            }

            let mut paragraph_lines = vec![current_line];
            i += 1;

            // 收集连续的非空行作为一个段落
            while i < lines.len() && !lines[i].trim().is_empty() && !is_table_start(&lines, i) {
                paragraph_lines.push(lines[i]);
                i += 1;
            }

            let paragraph = paragraph_lines.join("\n");

            // 如果段落本身超过块大小，需要按句子分割
            if paragraph.len() > chunk_size {
                // 按句子分割段落
                for sentence in paragraph.split(&['.', '。', '!', '?', '！', '？']) {
                    let sentence = sentence.trim();
                    if sentence.is_empty() {
                        continue;
                    }

                    let sentence_with_punct = format!("{}. ", sentence);

                    if current_size + sentence_with_punct.len() > chunk_size && current_size > 0 {
                        chunks.push(current_chunk.trim().to_string());
                        current_chunk = String::new();
                        current_size = 0;
                    }

                    current_chunk.push_str(&sentence_with_punct);
                    current_size += sentence_with_punct.len();
                }
            } else if !paragraph.trim().is_empty() {
                // 段落可以作为一个整体添加
                let paragraph_with_newlines = format!("{}\n\n", paragraph);

                if current_size + paragraph_with_newlines.len() > chunk_size && current_size > 0 {
                    chunks.push(current_chunk.trim().to_string());
                    current_chunk = String::new();
                    current_size = 0;
                }

                current_chunk.push_str(&paragraph_with_newlines);
                current_size += paragraph_with_newlines.len();
            }

            // 跳过空行
            while i < lines.len() && lines[i].trim().is_empty() {
                i += 1;
            }
        }
    }

    // 添加最后一个块
    if !current_chunk.trim().is_empty() {
        chunks.push(current_chunk.trim().to_string());
    }

    // 如果没有生成任何块（例如文本为空），返回包含原始文本的单个块
    if chunks.is_empty() && !text.is_empty() {
        chunks.push(text.to_string());
    }

    chunks
}

/// 检测是否是表格的开始
fn is_table_start(lines: &[&str], index: usize) -> bool {
    if index >= lines.len() {
        return false;
    }

    let line = lines[index].trim();

    // 检查当前行是否包含表格分隔符（如 |---|---|）
    if line.contains("|") {
        // 如果是分隔符行
        if line.contains("---") || line.contains("===") {
            info!("is_table_start({}): true - separator line", index);
            return true;
        }

        // 或者当前行和下一行都包含 |
        if index + 1 < lines.len() {
            let next_line = lines[index + 1].trim();
            if next_line.contains("|") {
                info!(
                    "is_table_start({}): true - current and next both have |",
                    index
                );
                return true;
            }
        }

        // 或者上一行也包含 |
        if index > 0 {
            let prev_line = lines[index - 1].trim();
            if prev_line.contains("|") {
                info!("is_table_start({}): true - prev has |", index);
                return true;
            }
        }

        info!(
            "is_table_start({}): false - has | but no adjacent | lines",
            index
        );
    }

    false
}

/// 收集完整的表格内容
fn collect_table(lines: &[&str], start: usize) -> (String, usize) {
    let mut table_lines = Vec::with_capacity(32);
    let mut i = start;

    // 向后找表格开始（如果start不是真正的开始）
    while i > 0 && lines[i - 1].trim().contains("|") {
        i -= 1;
    }

    // 收集所有表格行
    while i < lines.len() {
        let line = lines[i].trim();

        if line.is_empty() {
            // 遇到空行，检查是否表格结束
            if i + 1 < lines.len() && lines[i + 1].trim().contains("|") {
                // 下一行还是表格，空行可能是表格内部的（少见）
                i += 1;
                continue;
            } else {
                // 表格结束
                break;
            }
        }

        if line.contains("|") {
            table_lines.push(lines[i]);
            i += 1;
        } else {
            // 不包含 | 的行，表格结束
            break;
        }
    }

    let table_text = table_lines.join("\n");
    (table_text, i.saturating_sub(1))
}

/// 分割超大表格，每个块保留表头
///
/// 将大表格分成多个小块，每个块都包含表头（前2行），这样保持表格结构的可读性
fn split_large_table(table_text: &str, chunk_size: usize) -> Vec<String> {
    let lines: Vec<&str> = table_text.lines().collect();

    if lines.len() <= 2 {
        // 表格太小，直接返回
        return vec![table_text.to_string()];
    }

    let estimated_chunks = (table_text.len() / chunk_size).max(1);
    let mut chunks = Vec::with_capacity(estimated_chunks);

    // 前两行通常是表头和分隔符
    let header_lines = if lines.len() >= 2 {
        vec![lines[0], lines[1]]
    } else {
        vec![lines[0]]
    };

    let header_text = header_lines.join("\n");
    let header_size = header_text.len() + 1; // +1 for newline

    // 如果表头本身就超过chunk_size，只能硬切
    if header_size >= chunk_size {
        // 按固定行数分割
        let mut current = String::new();
        for (idx, line) in lines.iter().enumerate() {
            let line_with_newline = if idx == lines.len() - 1 {
                line.to_string()
            } else {
                format!("{}\n", line)
            };

            if current.len() + line_with_newline.len() > chunk_size && !current.is_empty() {
                chunks.push(current.trim().to_string());
                current = String::new();
            }

            current.push_str(&line_with_newline);
        }

        if !current.is_empty() {
            chunks.push(current.trim().to_string());
        }

        return chunks;
    }

    // 从第3行开始分块（保留表头）
    let mut current_chunk = header_text.clone();
    let mut current_size = header_size;

    for line in lines.iter().skip(2) {
        let row_with_newline = format!("\n{}", line);
        let row_size = row_with_newline.len();

        // 如果加上这一行会超出大小
        if current_size + row_size > chunk_size {
            // 保存当前块
            chunks.push(current_chunk.clone());

            // 开始新块，带表头
            current_chunk = format!("{}{}", header_text, row_with_newline);
            current_size = header_size + row_size;
        } else {
            current_chunk.push_str(&row_with_newline);
            current_size += row_size;
        }
    }

    // 添加最后一个块
    if current_chunk.len() > header_size {
        chunks.push(current_chunk);
    }

    // 如果没有生成任何块，返回原始表格
    if chunks.is_empty() {
        chunks.push(table_text.to_string());
    }

    chunks
}
