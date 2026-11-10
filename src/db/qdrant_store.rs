use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use qdrant_client::{
    Payload, Qdrant,
    qdrant::{
        Condition, CountPointsBuilder, CreateCollectionBuilder, CreateFieldIndexCollectionBuilder,
        DeletePointsBuilder, Direction, FieldType, Filter as QdrantClientFilter, OrderByBuilder,
        Query, QueryPointsBuilder, ScrollPointsBuilder, VectorParamsBuilder, points_selector,
    },
};
use rig::{
    DynModel, Embed,
    embeddings::EmbeddingsBuilder,
    operation::Embedding as EmbeddingOp,
    qdrant::{QdrantFilter, QdrantVectorStore},
    vector_store::{
        InsertDocuments, VectorStoreError, VectorStoreIndex,
        request::{VectorSearchRequest, DynamicSearchFilter},
    },
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tracing::{debug, info, warn};

use crate::config::QdrantConfig;

/// Erased embedding model handle used across the app.
pub type EmbeddingHandle = DynModel<EmbeddingOp>;

/// 文档结构
#[derive(Debug, Clone, Serialize, Deserialize, Embed, PartialEq)]
pub struct Document {
    pub id: String,
    pub base_id: String,
    pub chunk_index: Option<u32>,
    #[embed]
    pub content: String,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Document {
    pub fn new(
        id: String,
        base_id: String,
        chunk_index: Option<u32>,
        content: String,
        source: String,
        timestamp: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            base_id,
            chunk_index,
            content,
            source,
            created_at: timestamp,
            updated_at: timestamp,
        }
    }
}

#[derive(Clone)]
pub struct SerializableQdrantVectorStore {
    inner: Arc<QdrantVectorStore>,
}

impl SerializableQdrantVectorStore {
    pub fn new(inner: QdrantVectorStore) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn inner(&self) -> Arc<QdrantVectorStore> {
        Arc::clone(&self.inner)
    }
}

impl VectorStoreIndex for SerializableQdrantVectorStore {
    type Filter = QdrantFilter;

    fn top_n<T: DeserializeOwned + Send>(
        &self,
        req: VectorSearchRequest<Self::Filter>,
    ) -> impl std::future::Future<
        Output = Result<Vec<rig::vector_store::VectorSearchResult<T>>, VectorStoreError>,
    > + Send {
        let inner = self.inner();
        async move { inner.top_n::<T>(req).await }
    }

    fn top_n_ids(
        &self,
        req: VectorSearchRequest<Self::Filter>,
    ) -> impl std::future::Future<
        Output = Result<Vec<rig::vector_store::VectorSearchIdResult>, VectorStoreError>,
    > + Send {
        let inner = self.inner();
        async move { inner.top_n_ids(req).await }
    }
}

/// Qdrant 文档存储
#[derive(Clone)]
pub struct DocumentStore {
    config: QdrantConfig,
}

impl DocumentStore {
    pub fn new(config: QdrantConfig) -> Self {
        Self { config }
    }

    pub fn with_config(config: &QdrantConfig) -> Self {
        Self::new(config.clone())
    }

    fn client(&self) -> Result<Qdrant> {
        let mut builder = Qdrant::from_url(&self.config.url);
        if let Some(api_key) = &self.config.api_key {
            builder = builder.api_key(api_key.clone());
        }
        builder.build().context("Failed to build Qdrant client")
    }

    async fn ensure_collection(&self, client: &Qdrant, vector_size: usize) -> Result<()> {
        if client
            .collection_exists(&self.config.collection_name)
            .await
            .context("Failed to check Qdrant collection existence")?
        {
            return Ok(());
        }

        let size = vector_size.max(self.config.vector_size) as u64;
        info!(
            collection = %self.config.collection_name,
            vector_size = size,
            distance = ?self.config.distance,
            "Creating Qdrant collection"
        );

        client
            .create_collection(
                CreateCollectionBuilder::new(&self.config.collection_name)
                    .vectors_config(VectorParamsBuilder::new(size, self.config.distance)),
            )
            .await
            .context("Failed to create Qdrant collection")?;

        self.ensure_payload_indexes(client).await?;

        Ok(())
    }

    async fn ensure_payload_indexes(&self, client: &Qdrant) -> Result<()> {
        for (field, field_type) in [
            ("id", FieldType::Keyword),
            ("base_id", FieldType::Keyword),
            ("updated_at", FieldType::Datetime),
        ] {
            if let Err(err) = client
                .create_field_index(
                    CreateFieldIndexCollectionBuilder::new(
                        &self.config.collection_name,
                        field,
                        field_type,
                    )
                    .wait(true),
                )
                .await
                && !is_already_exists(&err)
            {
                warn!("Failed to create index on field '{}': {}", field, err);
            }
        }
        Ok(())
    }

    async fn collection_exists(&self, client: &Qdrant) -> Result<bool> {
        client
            .collection_exists(&self.config.collection_name)
            .await
            .context("Failed to check Qdrant collection existence")
    }

    async fn collection_count(&self, client: &Qdrant) -> Result<usize> {
        let response = client
            .count(
                CountPointsBuilder::new(&self.config.collection_name)
                    .exact(false)
                    .build(),
            )
            .await
            .context("Failed to count documents in Qdrant")?;

        Ok(response
            .result
            .map(|r| r.count as usize)
            .unwrap_or_default())
    }

    fn build_vector_store(&self, client: Qdrant, model: EmbeddingHandle) -> QdrantVectorStore {
        let query_params = QueryPointsBuilder::new(&self.config.collection_name)
            .with_payload(true)
            .with_vectors(false)
            .build();
        QdrantVectorStore::new(client, model, query_params)
    }

    fn build_filter_for_identifier(&self, identifier: &str) -> QdrantClientFilter {
        if identifier.ends_with("_CHUNKED") {
            let base_id = identifier.trim_end_matches("_CHUNKED");
            QdrantClientFilter::must([Condition::matches("base_id", base_id.to_string())])
        } else {
            QdrantClientFilter::must([Condition::matches("id", identifier.to_string())])
        }
    }

    fn deserialize_document(
        payload: HashMap<String, qdrant_client::qdrant::Value>,
    ) -> Result<Document> {
        let json_value: serde_json::Value = Payload::from(payload).into();
        serde_json::from_value(json_value)
            .context("Failed to deserialize document from Qdrant payload")
    }

    pub async fn create_vector_index(
        &self,
        embedding_model: EmbeddingHandle,
    ) -> Result<SerializableQdrantVectorStore> {
        let client = self.client()?;
        self.ensure_collection(&client, self.config.vector_size)
            .await?;

        let vector_store = self.build_vector_store(client, embedding_model);
        Ok(SerializableQdrantVectorStore::new(vector_store))
    }

    pub async fn search(
        &self,
        vector_index: &SerializableQdrantVectorStore,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(f64, Document)>> {
        let req = VectorSearchRequest::builder()
            .query(query)
            .samples(limit as u64)
            .build();

        let mapped = req.map_filter(|filter| {
            QdrantFilter::from_dynamic_filter(filter).expect("canonical filter maps to QdrantFilter")
        });

        let results = vector_index
            .top_n::<Document>(mapped)
            .await
            .context("Vector search on Qdrant failed")?;

        Ok(results
            .into_iter()
            .map(|hit| (hit.score, hit.document))
            .collect())
    }

    pub async fn count_documents_async(&self) -> Result<usize> {
        let client = self.client()?;
        if !self.collection_exists(&client).await? {
            return Ok(0);
        }

        self.collection_count(&client).await
    }

    pub async fn add_documents_with_embeddings(
        &self,
        documents: Vec<Document>,
        embedding_model: EmbeddingHandle,
    ) -> Result<()> {
        if documents.is_empty() {
            debug!("No documents to add to Qdrant, skipping");
            return Ok(());
        }

        let client = self.client()?;
        self.ensure_collection(&client, self.config.vector_size)
            .await?;

        let vector_store = self.build_vector_store(client, embedding_model.clone());
        let total = documents.len();
        info!(
            collection = %self.config.collection_name,
            total = total,
            "Adding documents to Qdrant"
        );
        // documents 分批插入, 每批最多10个文档
        for chunk in documents.chunks(10) {
            let embeddings = EmbeddingsBuilder::new(embedding_model.clone())
                .documents(chunk.to_vec())
                .context("Failed to create embeddings builder")?
                .build()
                .await
                .context("Failed to create embeddings")?;

            vector_store
                .insert_documents(embeddings)
                .await
                .context("Failed to insert documents into Qdrant")?;
        }

        Ok(())
    }

    pub async fn get_document(&self, id: &str) -> Result<Option<Document>> {
        let client = self.client()?;
        if !self.collection_exists(&client).await? {
            return Ok(None);
        }

        let response = client
            .scroll(
                ScrollPointsBuilder::new(&self.config.collection_name)
                    .filter(self.build_filter_for_identifier(id))
                    .with_payload(true)
                    .with_vectors(false)
                    .limit(1)
                    .build(),
            )
            .await
            .context("Failed to retrieve document from Qdrant")?;

        if let Some(point) = response.result.into_iter().next() {
            let doc = Self::deserialize_document(point.payload)?;
            return Ok(Some(doc));
        }

        Ok(None)
    }

    pub async fn list_documents_paginated(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<Document>, usize)> {
        let client = self.client()?;
        if !self.collection_exists(&client).await? {
            return Ok((Vec::new(), 0));
        }

        let total = self.collection_count(&client).await?;
        if total == 0 {
            return Ok((Vec::new(), 0));
        }

        let safe_limit = limit.clamp(1, 1000);

        let order_by = OrderByBuilder::new("updated_at")
            .direction(Direction::Desc as i32)
            .build();

        let response = client
            .query(
                QueryPointsBuilder::new(&self.config.collection_name)
                    .query(Query::new_order_by(order_by))
                    .offset(offset as u64)
                    .limit(safe_limit as u64)
                    .with_payload(true)
                    .with_vectors(false)
                    .build(),
            )
            .await
            .context("Failed to query documents from Qdrant")?;

        let documents = response
            .result
            .into_iter()
            .filter_map(|point| match Self::deserialize_document(point.payload) {
                Ok(doc) => Some(doc),
                Err(err) => {
                    warn!("Failed to deserialize document payload: {}", err);
                    None
                }
            })
            .collect();

        Ok((documents, total))
    }

    pub async fn delete_document(&self, identifier: &str) -> Result<()> {
        let client = self.client()?;
        if !self.collection_exists(&client).await? {
            return Ok(());
        }

        let filter = self.build_filter_for_identifier(identifier);
        let selector = points_selector::PointsSelectorOneOf::Filter(filter);

        client
            .delete_points(
                DeletePointsBuilder::new(&self.config.collection_name)
                    .points(selector)
                    .wait(true)
                    .build(),
            )
            .await
            .context("Failed to delete document(s) from Qdrant")?;

        Ok(())
    }

    pub async fn reset_table(&self) -> Result<()> {
        let client = self.client()?;
        if client
            .collection_exists(&self.config.collection_name)
            .await
            .context("Failed to check Qdrant collection existence")?
        {
            client
                .delete_collection(&self.config.collection_name)
                .await
                .context("Failed to drop Qdrant collection")?;
            info!(collection = %self.config.collection_name, "Dropped Qdrant collection");
        }

        Ok(())
    }
}

fn is_already_exists(err: &qdrant_client::QdrantError) -> bool {
    err.to_string().contains("already exists")
}
