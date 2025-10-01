use anyhow::Result;
use arrow_array::{
    ArrayRef, FixedSizeListArray, RecordBatch, RecordBatchIterator, StringArray,
    TimestampMillisecondArray, types::Float64Type,
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use lancedb::arrow::arrow_schema::{DataType, Field, Fields, Schema, TimeUnit};
use lancedb::query::{ExecutableQuery, QueryBase};
use rig::embeddings::Embedding;
use rig::{
    Embed, OneOrMany,
    embeddings::{EmbeddingModel, EmbeddingsBuilder},
    vector_store::request::VectorSearchRequest,
    vector_store::{VectorStoreIndex, VectorStoreIndexDyn},
};
use rig_lancedb::{LanceDbVectorIndex, SearchParams};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize, Embed, PartialEq)]
pub struct StoredDocument {
    pub id: String,
    #[embed]
    pub content: String,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

impl Default for StoredDocument {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: String::new(),
            content: String::new(),
            source: String::new(),
            created_at: now,
            updated_at: now,
            tags: Vec::new(),
        }
    }
}

impl StoredDocument {
    pub fn new(content: String, source: String) -> Self {
        let now = Utc::now();
        Self {
            id: nanoid::nanoid!(),
            content,
            source,
            created_at: now,
            updated_at: now,
            tags: Vec::new(),
        }
    }

    /// 更新文档内容，同时更新updated_at时间戳
    pub fn update_content(mut self, content: String) -> Self {
        self.content = content;
        self.updated_at = Utc::now();
        self
    }

    /// 更新文档来源，同时更新updated_at时间戳
    pub fn update_source(mut self, source: String) -> Self {
        self.source = source;
        self.updated_at = Utc::now();
        self
    }

    pub fn with_id(mut self, id: String) -> Self {
        self.id = id;
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

/// LanceDB 向量存储，提供完整的向量搜索功能
#[derive(Clone)]
pub struct DocumentStore {
    pub db_path: String,
    pub table_name: String,
    vector_index: Arc<RwLock<Option<Arc<dyn VectorStoreIndexDyn>>>>,
}

impl DocumentStore {
    pub async fn new(db_path: &str, table_name: &str) -> Result<Self> {
        Ok(Self {
            db_path: db_path.to_string(),
            table_name: table_name.to_string(),
            vector_index: Arc::new(RwLock::new(None)),
        })
    }

    /// 检查并加载已存在的向量索引
    pub async fn load_existing_index<M>(&self, embedding_model: M) -> Result<bool>
    where
        M: EmbeddingModel + Clone + Send + Sync + 'static,
    {
        let db = lancedb::connect(&self.db_path).execute().await?;

        // 检查表是否存在
        if !db.table_names().execute().await?.contains(&self.table_name) {
            tracing::info!(
                "📋 Table '{}' does not exist, will create new one when documents are added",
                self.table_name
            );
            return Ok(false);
        }

        // 打开已存在的表
        let table = db.open_table(&self.table_name).execute().await?;

        // 检查表是否有数据
        let count_result = table.count_rows(None).await;
        match count_result {
            Ok(count) => {
                if count == 0 {
                    tracing::info!("📋 Table '{}' exists but is empty", self.table_name);
                    return Ok(false);
                }

                tracing::info!(
                    "📋 Found existing table '{}' with {} documents",
                    self.table_name,
                    count
                );

                // 创建向量索引连接到已存在的表
                let search_params = SearchParams::default();
                let vector_index =
                    LanceDbVectorIndex::new(table, embedding_model, "id", search_params).await?;

                *self.vector_index.write().unwrap() = Some(Arc::new(vector_index));

                tracing::info!(
                    "✅ Successfully loaded existing vector index with {} documents",
                    count
                );
                Ok(true)
            }
            Err(e) => {
                tracing::warn!("⚠️ Failed to count rows in existing table: {}", e);
                Ok(false)
            }
        }
    }

    /// 重置数据库表（删除现有表以处理schema变化）
    pub async fn reset_table(&self) -> Result<()> {
        let db = lancedb::connect(&self.db_path).execute().await?;

        // 检查表是否存在
        if db.table_names().execute().await?.contains(&self.table_name) {
            // 删除现有表
            db.drop_table(&self.table_name).await?;
            tracing::info!(
                "🗑️ Dropped existing table '{}' due to schema changes",
                self.table_name
            );
        }

        // 清除向量索引
        *self.vector_index.write().unwrap() = None;

        Ok(())
    }

    /// 创建 LanceDB Schema
    fn schema(&self, dims: usize) -> Schema {
        Schema::new(Fields::from(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("source", DataType::Utf8, false),
            Field::new(
                "created_at",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                false,
            ),
            Field::new(
                "updated_at",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                false,
            ),
            Field::new("tags", DataType::Utf8, false), // JSON string for tags
            Field::new(
                "embedding",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float64, true)),
                    dims as i32,
                ),
                false,
            ),
        ]))
    }

    /// 将 StoredDocument 和 embeddings 转换为 RecordBatch
    fn as_record_batch(
        &self,
        records: Vec<(StoredDocument, OneOrMany<Embedding>)>,
        dims: usize,
    ) -> Result<RecordBatch> {
        let ids = StringArray::from_iter_values(records.iter().map(|(doc, _)| doc.id.clone()));

        let contents =
            StringArray::from_iter_values(records.iter().map(|(doc, _)| doc.content.clone()));

        let sources =
            StringArray::from_iter_values(records.iter().map(|(doc, _)| doc.source.clone()));

        let created_at_timestamps = TimestampMillisecondArray::from_iter_values(
            records
                .iter()
                .map(|(doc, _)| doc.created_at.timestamp_millis()),
        );

        let updated_at_timestamps = TimestampMillisecondArray::from_iter_values(
            records
                .iter()
                .map(|(doc, _)| doc.updated_at.timestamp_millis()),
        );

        let tags = StringArray::from_iter_values(
            records
                .iter()
                .map(|(doc, _)| serde_json::to_string(&doc.tags).unwrap_or_default()),
        );

        // 获取实际的embedding维度
        let actual_dims = if let Some((_, emb)) = records.first() {
            emb.first().vec.len()
        } else {
            dims
        };

        let embeddings = FixedSizeListArray::from_iter_primitive::<Float64Type, _, _>(
            records
                .into_iter()
                .map(|(_, embeddings)| {
                    Some(
                        embeddings
                            .first()
                            .vec
                            .into_iter()
                            .map(Some)
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>(),
            actual_dims as i32,
        );

        RecordBatch::try_from_iter(vec![
            ("id", Arc::new(ids) as ArrayRef),
            ("content", Arc::new(contents) as ArrayRef),
            ("source", Arc::new(sources) as ArrayRef),
            ("created_at", Arc::new(created_at_timestamps) as ArrayRef),
            ("updated_at", Arc::new(updated_at_timestamps) as ArrayRef),
            ("tags", Arc::new(tags) as ArrayRef),
            ("embedding", Arc::new(embeddings) as ArrayRef),
        ])
        .map_err(|e| anyhow::anyhow!("Failed to create RecordBatch: {}", e))
    }

    /// 初始化文档存储
    pub async fn initialize_with_embeddings<M>(
        &self,
        documents: Vec<StoredDocument>,
        embedding_model: M,
    ) -> Result<()>
    where
        M: EmbeddingModel + Clone + Send + Sync + 'static,
    {
        if documents.is_empty() {
            return Ok(());
        }

        // 检查并处理schema兼容性
        if let Err(e) = self.check_schema_compatibility().await {
            tracing::warn!(
                "🔄 Schema incompatibility detected, resetting table... {}",
                e
            );
            self.reset_table().await?;
        }

        // 生成embeddings
        let embeddings = EmbeddingsBuilder::new(embedding_model.clone())
            .documents(documents.clone())?
            .build()
            .await?;

        // 创建数据库连接和表
        let db = lancedb::connect(&self.db_path).execute().await?;

        let table = if db.table_names().execute().await?.contains(&self.table_name) {
            db.open_table(&self.table_name).execute().await?
        } else {
            // 获取实际的embedding维度
            let actual_dims = if let Some((_, emb)) = embeddings.first() {
                emb.first().vec.len()
            } else {
                embedding_model.ndims()
            };

            let record_batch = self.as_record_batch(embeddings, actual_dims)?;
            db.create_table(
                &self.table_name,
                RecordBatchIterator::new(
                    vec![Ok(record_batch)],
                    Arc::new(self.schema(actual_dims)),
                ),
            )
            .execute()
            .await?
        };

        // 创建向量索引
        let search_params = SearchParams::default();
        let vector_index =
            LanceDbVectorIndex::new(table, embedding_model, "id", search_params).await?;

        *self.vector_index.write().unwrap() = Some(Arc::new(vector_index));
        Ok(())
    }

    /// 检查schema兼容性
    async fn check_schema_compatibility(&self) -> Result<()> {
        let db = lancedb::connect(&self.db_path).execute().await?;

        if !db.table_names().execute().await?.contains(&self.table_name) {
            return Ok(()); // 表不存在，没有兼容性问题
        }

        let table = db.open_table(&self.table_name).execute().await?;

        // 尝试查询一个文档来检测schema
        match table.query().limit(1).execute().await {
            Ok(mut stream) => {
                if let Ok(Some(batch)) = stream.try_next().await {
                    let schema = batch.schema();

                    // 检查是否有新的字段
                    let has_created_at = schema.column_with_name("created_at").is_some();
                    let has_updated_at = schema.column_with_name("updated_at").is_some();
                    let has_old_timestamp = schema.column_with_name("timestamp").is_some();

                    if has_old_timestamp && (!has_created_at || !has_updated_at) {
                        tracing::info!(
                            "🔄 Detected old schema with 'timestamp' field, needs migration"
                        );
                        return Err(anyhow::anyhow!("Schema migration needed"));
                    }
                }
                Ok(())
            }
            Err(e) => {
                tracing::warn!("Failed to check schema compatibility: {}", e);
                Err(anyhow::anyhow!("Schema check failed: {}", e))
            }
        }
    }

    /// 添加文档
    pub async fn add_documents_with_embeddings<M>(
        &self,
        documents: Vec<StoredDocument>,
        embedding_model: M,
    ) -> Result<()>
    where
        M: EmbeddingModel + Clone + Send + Sync + 'static,
    {
        if documents.is_empty() {
            return Ok(());
        }

        // 检查并处理schema兼容性
        if let Err(e) = self.check_schema_compatibility().await {
            tracing::warn!(
                "🔄 Schema incompatibility detected, resetting table... {}",
                e
            );
            self.reset_table().await?;
        }

        // 生成embeddings
        let embeddings = EmbeddingsBuilder::new(embedding_model.clone())
            .documents(documents.clone())?
            .build()
            .await?;

        // 创建数据库连接
        let db = lancedb::connect(&self.db_path).execute().await?;

        // 获取实际的embedding维度
        let actual_dims = if let Some((_, emb)) = embeddings.first() {
            emb.first().vec.len()
        } else {
            embedding_model.ndims()
        };

        let record_batch = self.as_record_batch(embeddings, actual_dims)?;

        // 检查表是否存在，如果不存在则创建
        let table = if db.table_names().execute().await?.contains(&self.table_name) {
            // 表存在，添加到现有表
            let table = db.open_table(&self.table_name).execute().await?;
            let batch_reader = RecordBatchIterator::new(
                vec![Ok(record_batch)],
                Arc::new(self.schema(actual_dims)),
            );
            table.add(batch_reader).execute().await?;
            table
        } else {
            // 表不存在，创建新表
            db.create_table(
                &self.table_name,
                RecordBatchIterator::new(
                    vec![Ok(record_batch)],
                    Arc::new(self.schema(actual_dims)),
                ),
            )
            .execute()
            .await?
        };

        // 更新向量索引
        let search_params = SearchParams::default();
        let vector_index =
            LanceDbVectorIndex::new(table, embedding_model, "id", search_params).await?;

        *self.vector_index.write().unwrap() = Some(Arc::new(vector_index));
        Ok(())
    }

    /// 简化的添加文档方法
    pub async fn add_document(&self, _document: StoredDocument) -> Result<()> {
        anyhow::bail!("请使用 add_documents_with_embeddings 方法")
    }

    /// 简化的批量添加文档方法
    pub async fn add_documents(&self, _documents: Vec<StoredDocument>) -> Result<()> {
        anyhow::bail!("请使用 add_documents_with_embeddings 方法")
    }

    /// 根据ID获取文档
    pub async fn get_document(&self, id: &str) -> Result<Option<StoredDocument>> {
        let db = lancedb::connect(&self.db_path).execute().await?;

        // 检查表是否存在
        if !db.table_names().execute().await?.contains(&self.table_name) {
            return Ok(None);
        }

        let table = db.open_table(&self.table_name).execute().await?;

        // 使用 only_if 查询查找指定ID的文档
        match table
            .query()
            .only_if(format!("id = '{}'", id))
            .limit(1)
            .execute()
            .await
        {
            Ok(stream) => {
                let batches: Vec<RecordBatch> = stream.try_collect().await?;

                // 简化的数据提取方法
                for batch in batches {
                    if batch.num_rows() > 0 {
                        // 手动提取第一行数据
                        if let (
                            Some(id_col),
                            Some(content_col),
                            Some(source_col),
                            Some(created_at_col),
                            Some(updated_at_col),
                            Some(tags_col),
                        ) = (
                            batch.column(0).as_any().downcast_ref::<StringArray>(),
                            batch.column(1).as_any().downcast_ref::<StringArray>(),
                            batch.column(2).as_any().downcast_ref::<StringArray>(),
                            batch
                                .column(3)
                                .as_any()
                                .downcast_ref::<TimestampMillisecondArray>(),
                            batch
                                .column(4)
                                .as_any()
                                .downcast_ref::<TimestampMillisecondArray>(),
                            batch.column(5).as_any().downcast_ref::<StringArray>(),
                        ) {
                            let doc_id = id_col.value(0);
                            let content = content_col.value(0);
                            let source = source_col.value(0);
                            let tags_json = tags_col.value(0);
                            let created_at_ms = created_at_col.value(0);
                            let updated_at_ms = updated_at_col.value(0);
                            let created_at = DateTime::from_timestamp_millis(created_at_ms)
                                .unwrap_or_else(Utc::now);
                            let updated_at = DateTime::from_timestamp_millis(updated_at_ms)
                                .unwrap_or_else(Utc::now);
                            let tags: Vec<String> =
                                serde_json::from_str(tags_json).unwrap_or_else(|_| Vec::new());

                            return Ok(Some(StoredDocument {
                                id: doc_id.to_string(),
                                content: content.to_string(),
                                source: source.to_string(),
                                created_at,
                                updated_at,
                                tags,
                            }));
                        }
                    }
                }
                Ok(None)
            }
            Err(e) => {
                tracing::warn!("Query failed for id {}: {}", id, e);
                Ok(None)
            }
        }
    }

    /// 获取所有文档
    pub async fn list_documents(&self) -> Result<Vec<StoredDocument>> {
        let db = lancedb::connect(&self.db_path).execute().await?;

        // 检查表是否存在
        if !db.table_names().execute().await?.contains(&self.table_name) {
            return Ok(Vec::new());
        }

        let table = db.open_table(&self.table_name).execute().await?;

        // 获取所有文档，限制数量避免内存问题
        match table.query().limit(1000).execute().await {
            Ok(stream) => {
                let batches: Vec<RecordBatch> = stream.try_collect().await?;
                let mut documents = Vec::new();

                // 简化的数据提取方法
                for batch in batches {
                    if batch.num_rows() > 0 {
                        // 手动提取数据
                        if let (
                            Some(id_col),
                            Some(content_col),
                            Some(source_col),
                            Some(created_at_col),
                            Some(updated_at_col),
                            Some(tags_col),
                        ) = (
                            batch.column(0).as_any().downcast_ref::<StringArray>(),
                            batch.column(1).as_any().downcast_ref::<StringArray>(),
                            batch.column(2).as_any().downcast_ref::<StringArray>(),
                            batch
                                .column(3)
                                .as_any()
                                .downcast_ref::<TimestampMillisecondArray>(),
                            batch
                                .column(4)
                                .as_any()
                                .downcast_ref::<TimestampMillisecondArray>(),
                            batch.column(5).as_any().downcast_ref::<StringArray>(),
                        ) {
                            for row_idx in 0..batch.num_rows() {
                                let doc_id = id_col.value(row_idx);
                                let content = content_col.value(row_idx);
                                let source = source_col.value(row_idx);
                                let tags_json = tags_col.value(row_idx);
                                let created_at_ms = created_at_col.value(row_idx);
                                let updated_at_ms = updated_at_col.value(row_idx);
                                let created_at = DateTime::from_timestamp_millis(created_at_ms)
                                    .unwrap_or_else(Utc::now);
                                let updated_at = DateTime::from_timestamp_millis(updated_at_ms)
                                    .unwrap_or_else(Utc::now);
                                let tags: Vec<String> =
                                    serde_json::from_str(tags_json).unwrap_or_else(|_| Vec::new());

                                documents.push(StoredDocument {
                                    id: doc_id.to_string(),
                                    content: content.to_string(),
                                    source: source.to_string(),
                                    created_at,
                                    updated_at,
                                    tags,
                                });
                            }
                        }
                    }
                }

                // 按更新时间降序排序
                documents.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
                Ok(documents)
            }
            Err(e) => {
                tracing::warn!("Failed to query documents: {}", e);
                Ok(Vec::new())
            }
        }
    }

    /// 分页获取文档
    pub async fn list_documents_paginated(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<StoredDocument>, usize)> {
        let db = lancedb::connect(&self.db_path).execute().await?;

        // 检查表是否存在
        if !db.table_names().execute().await?.contains(&self.table_name) {
            return Ok((Vec::new(), 0));
        }

        let table = db.open_table(&self.table_name).execute().await?;

        // 计算总数
        let total = table.count_rows(None).await.unwrap_or(0) as usize;

        // 保护性处理，避免过大limit
        let safe_limit = limit.clamp(1, 1000);
        let upto = offset.saturating_add(safe_limit);

        // LanceDB 查询不支持 offset，这里通过 limit(offset+limit) 后内存切片
        match table.query().limit(upto).execute().await {
            Ok(stream) => {
                let batches: Vec<RecordBatch> = stream.try_collect().await?;
                let mut documents = Vec::new();

                for batch in batches {
                    if batch.num_rows() > 0
                        && let (
                            Some(id_col),
                            Some(content_col),
                            Some(source_col),
                            Some(created_at_col),
                            Some(updated_at_col),
                            Some(tags_col),
                        ) = (
                            batch.column(0).as_any().downcast_ref::<StringArray>(),
                            batch.column(1).as_any().downcast_ref::<StringArray>(),
                            batch.column(2).as_any().downcast_ref::<StringArray>(),
                            batch
                                .column(3)
                                .as_any()
                                .downcast_ref::<TimestampMillisecondArray>(),
                            batch
                                .column(4)
                                .as_any()
                                .downcast_ref::<TimestampMillisecondArray>(),
                            batch.column(5).as_any().downcast_ref::<StringArray>(),
                        )
                    {
                        for row_idx in 0..batch.num_rows() {
                            let doc_id = id_col.value(row_idx);
                            let content = content_col.value(row_idx);
                            let source = source_col.value(row_idx);
                            let tags_json = tags_col.value(row_idx);
                            let created_at_ms = created_at_col.value(row_idx);
                            let updated_at_ms = updated_at_col.value(row_idx);
                            let created_at = DateTime::from_timestamp_millis(created_at_ms)
                                .unwrap_or_else(Utc::now);
                            let updated_at = DateTime::from_timestamp_millis(updated_at_ms)
                                .unwrap_or_else(Utc::now);
                            let tags: Vec<String> =
                                serde_json::from_str(tags_json).unwrap_or_else(|_| Vec::new());

                            documents.push(StoredDocument {
                                id: doc_id.to_string(),
                                content: content.to_string(),
                                source: source.to_string(),
                                created_at,
                                updated_at,
                                tags,
                            });
                        }
                    }
                }

                // 按更新时间降序排序
                documents.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

                // 应用 offset/limit 切片
                let start = offset.min(documents.len());
                let end = (start + safe_limit).min(documents.len());
                let page = documents[start..end].to_vec();

                Ok((page, total))
            }
            Err(e) => {
                tracing::warn!("Failed to query documents (paginated): {}", e);
                Ok((Vec::new(), total))
            }
        }
    }

    /// 删除文档
    pub async fn delete_document(&self, id: &str) -> Result<()> {
        let db = lancedb::connect(&self.db_path).execute().await?;
        let table = db.open_table(&self.table_name).execute().await?;
        table.delete(&format!("id = '{}'", id)).await?;
        Ok(())
    }

    /// 向量相似性搜索 - 使用LanceDB的真正向量搜索
    pub async fn vector_search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(StoredDocument, f32)>> {
        let vector_index = self.vector_index.read().unwrap().clone();
        match vector_index {
            Some(ref vector_index) => {
                let req = VectorSearchRequest::builder()
                    .query(query)
                    .samples(limit as u64)
                    .build()?;
                let results = vector_index.top_n_ids(req).await?;
                let mut documents = Vec::new();
                for (score, doc_id) in results {
                    // 从数据库获取真实的文档内容
                    tracing::info!(
                        "🔍 Attempting to retrieve document with ID: {} (score: {})",
                        doc_id,
                        score
                    );
                    match self.get_document(&doc_id).await {
                        Ok(Some(doc)) => {
                            tracing::info!(
                                "✅ Successfully retrieved document: {} chars",
                                doc.content.len()
                            );
                            documents.push((doc, score as f32));
                        }
                        Ok(None) => {
                            tracing::warn!("⚠️ Document not found for ID: {}", doc_id);
                        }
                        Err(e) => {
                            tracing::error!(
                                "❌ Failed to retrieve document content for ID: {}: {}",
                                doc_id,
                                e
                            );
                        }
                    }
                }
                Ok(documents)
            }
            None => {
                // 如果没有向量索引，返回空结果
                Ok(Vec::new())
            }
        }
    }

    /// 获取文档数量
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        // 异步获取实际文档数量比较复杂，这里使用一个合理的默认值
        // 在实际应用中，可以考虑缓存文档数量或使用同步方法
        if self.vector_index.read().unwrap().is_some() {
            // 返回一个足够大的值，确保RAG能正常工作
            100
        } else {
            1
        }
    }

    /// 异步获取实际文档数量
    pub async fn count_documents(&self) -> Result<usize> {
        let docs = self.list_documents().await?;
        Ok(docs.len())
    }

    /// 获取向量索引（为了兼容现有代码）
    pub fn get_vector_index(&self) -> Option<&Self> {
        if self.vector_index.read().unwrap().is_some() {
            Some(self)
        } else {
            None
        }
    }
}

// 实现 VectorStoreIndex trait 使其可以被 dynamic_context 使用
impl VectorStoreIndex for DocumentStore {
    async fn top_n_ids(
        &self,
        req: VectorSearchRequest,
    ) -> Result<Vec<(f64, String)>, rig::vector_store::VectorStoreError> {
        let vector_index_opt = {
            let guard = self.vector_index.read().unwrap();
            guard.clone()
        };

        if let Some(vector_index) = vector_index_opt {
            let results = vector_index
                .top_n_ids(req)
                .await
                .map_err(|e| rig::vector_store::VectorStoreError::DatastoreError(Box::new(e)))?;

            Ok(results)
        } else {
            Ok(Vec::new())
        }
    }

    async fn top_n<T: for<'a> serde::Deserialize<'a> + Send>(
        &self,
        req: VectorSearchRequest,
    ) -> Result<Vec<(f64, String, T)>, rig::vector_store::VectorStoreError> {
        let vector_index_opt = {
            let guard = self.vector_index.read().unwrap();
            guard.clone()
        };

        if let Some(vector_index) = vector_index_opt {
            let results = vector_index
                .top_n_ids(req)
                .await
                .map_err(|e| rig::vector_store::VectorStoreError::DatastoreError(Box::new(e)))?;

            // 获取实际文档内容并反序列化
            let mut documents = Vec::new();

            for (score, doc_id) in results {
                // 从数据库获取文档
                tracing::info!(
                    "🔍 RAG: Attempting to retrieve document with ID: {} (score: {})",
                    doc_id,
                    score
                );
                match self.get_document(&doc_id).await {
                    Ok(Some(doc)) => {
                        tracing::info!(
                            "✅ RAG: Successfully retrieved document: {} chars",
                            doc.content.len()
                        );
                        // let preview = if doc.content.len() > 200 {
                        //     doc.content.chars().take(200).collect::<String>()
                        // } else {
                        //     doc.content.clone()
                        // };
                        // tracing::info!("📄 RAG: Document content preview: {}", preview);
                        // 尝试将文档反序列化为T类型
                        if let Ok(serialized) = serde_json::to_string(&doc) {
                            // let json_preview = if serialized.len() > 300 {
                            //     serialized.chars().take(300).collect::<String>()
                            // } else {
                            //     serialized.clone()
                            // };
                            // tracing::info!("📦 RAG: Serialized format: {}", json_preview);
                            if let Ok(deserialized) = serde_json::from_str::<T>(&serialized) {
                                // tracing::info!("✅ RAG: Document serialization successful");
                                documents.push((score, doc_id, deserialized));
                            } else {
                                tracing::warn!(
                                    "⚠️ RAG: Document deserialization failed for ID: {}",
                                    doc_id
                                );
                            }
                        } else {
                            tracing::warn!(
                                "⚠️ RAG: Document serialization failed for ID: {}",
                                doc_id
                            );
                        }
                    }
                    Ok(None) => {
                        tracing::warn!("⚠️ RAG: Document not found for ID: {}", doc_id);
                    }
                    Err(e) => {
                        tracing::error!(
                            "❌ RAG: Failed to retrieve document content for ID: {}: {}",
                            doc_id,
                            e
                        );
                    }
                }
            }

            Ok(documents)
        } else {
            Ok(Vec::new())
        }
    }
}

// 也为 &DocumentStore 实现 trait
impl VectorStoreIndex for &DocumentStore {
    async fn top_n_ids(
        &self,
        req: VectorSearchRequest,
    ) -> Result<Vec<(f64, String)>, rig::vector_store::VectorStoreError> {
        VectorStoreIndex::top_n_ids(*self, req).await
    }

    async fn top_n<T: for<'a> serde::Deserialize<'a> + Send>(
        &self,
        req: VectorSearchRequest,
    ) -> Result<Vec<(f64, String, T)>, rig::vector_store::VectorStoreError> {
        VectorStoreIndex::top_n(*self, req).await
    }
}

// 创建一个包装类型来避免孤儿规则
#[derive(Clone)]
pub struct DocumentStoreWrapper(pub Arc<DocumentStore>);

impl VectorStoreIndex for DocumentStoreWrapper {
    async fn top_n_ids(
        &self,
        req: VectorSearchRequest,
    ) -> Result<Vec<(f64, String)>, rig::vector_store::VectorStoreError> {
        VectorStoreIndex::top_n_ids(self.0.as_ref(), req).await
    }

    async fn top_n<T: for<'a> serde::Deserialize<'a> + Send>(
        &self,
        req: VectorSearchRequest,
    ) -> Result<Vec<(f64, String, T)>, rig::vector_store::VectorStoreError> {
        VectorStoreIndex::top_n(self.0.as_ref(), req).await
    }
}
