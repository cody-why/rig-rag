use std::sync::Arc;

use anyhow::{Context, Result};
use arrow_array::{ArrayRef, FixedSizeListArray, RecordBatch, RecordBatchIterator, StringArray, TimestampMillisecondArray, types::Float64Type};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use lancedb::arrow::arrow_schema::{DataType, Field, Fields, Schema, TimeUnit};
use lancedb::query::{ExecutableQuery, QueryBase};
use rig::embeddings::Embedding;
use rig::{Embed, OneOrMany, embeddings::{EmbeddingModel, EmbeddingsBuilder}, vector_store::VectorStoreIndex, vector_store::request::VectorSearchRequest};
use rig_lancedb::{LanceDbVectorIndex, SearchParams};
use serde::{Deserialize, Deserializer, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// 文档结构
#[derive(Debug, Clone, Serialize, Embed, PartialEq)]
pub struct Document {
    pub id: String,
    #[embed]
    pub content: String,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // pub tags: Vec<String>,
}

impl Document {
    pub fn new(content: String, source: String) -> Self {
        let now = Utc::now();
        Self {
            id: nanoid::nanoid!(),
            content,
            source,
            created_at: now,
            updated_at: now,
        }
    }
}

impl<'de> Deserialize<'de> for Document {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct DocumentHelper {
            id: String,
            content: String,
            source: String,
            created_at: i64,
            updated_at: i64,
        }

        let helper = DocumentHelper::deserialize(deserializer)?;

        // 将毫秒时间戳转换为 DateTime<Utc>
        let created_at = DateTime::from_timestamp_millis(helper.created_at).unwrap_or_else(|| {
            warn!(
                "Invalid created_at timestamp: {}, using current time",
                helper.created_at
            );
            Utc::now()
        });
        let updated_at = DateTime::from_timestamp_millis(helper.updated_at).unwrap_or_else(|| {
            warn!(
                "Invalid updated_at timestamp: {}, using current time",
                helper.updated_at
            );
            Utc::now()
        });

        Ok(Document {
            id: helper.id,
            content: helper.content,
            source: helper.source,
            created_at,
            updated_at,
        })
    }
}

/// LanceDB 向量存储
#[derive(Clone)]
pub struct DocumentStore<M: EmbeddingModel> {
    vector_index: Arc<RwLock<Option<LanceDbVectorIndex<M>>>>,
    db_path: String,
    table_name: String,
}

impl<M: EmbeddingModel> DocumentStore<M> {
    pub fn new(db_path: &str, table_name: &str) -> Self {
        Self {
            db_path: db_path.to_string(),
            table_name: table_name.to_string(),
            vector_index: Arc::new(RwLock::new(None)),
        }
    }

    /// 加载已存在的向量索引
    pub async fn load_existing_index(&self, embedding_model: M) -> Result<bool>
    where
        M: Clone + Send + Sync + 'static,
    {
        let db = lancedb::connect(&self.db_path)
            .execute()
            .await
            .context("Failed to connect to LanceDB")?;

        let table_exists = db
            .table_names()
            .execute()
            .await
            .context("Failed to list table names")?
            .contains(&self.table_name);

        if !table_exists {
            info!(
                "📋 Table '{}' does not exist, will create new one when documents are added",
                self.table_name
            );
            return Ok(false);
        }

        let table = db
            .open_table(&self.table_name)
            .execute()
            .await
            .context("Failed to open table")?;

        let search_params = SearchParams::default().column("embedding");
        let vector_index = LanceDbVectorIndex::new(table, embedding_model, "id", search_params)
            .await
            .context("Failed to create vector index")?;

        *self.vector_index.write().await = Some(vector_index);
        Ok(true)
    }

    /// 从 RecordBatch 解析 Document
    fn parse_document_from_batch(batch: &RecordBatch, row_idx: usize) -> Result<Document> {
        if row_idx >= batch.num_rows() {
            return Err(anyhow::anyhow!(
                "Row index {} out of bounds ({} rows)",
                row_idx,
                batch.num_rows()
            ));
        }

        let get_string_column = |col_idx: usize, name: &str| -> Result<&StringArray> {
            batch
                .column(col_idx)
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| anyhow::anyhow!("Invalid {} column", name))
        };

        let get_timestamp_column =
            |col_idx: usize, name: &str| -> Result<&TimestampMillisecondArray> {
                batch
                    .column(col_idx)
                    .as_any()
                    .downcast_ref::<TimestampMillisecondArray>()
                    .ok_or_else(|| anyhow::anyhow!("Invalid {} column", name))
            };

        let id_col = get_string_column(0, "id")?;
        let content_col = get_string_column(1, "content")?;
        let source_col = get_string_column(2, "source")?;
        let created_at_col = get_timestamp_column(3, "created_at")?;
        let updated_at_col = get_timestamp_column(4, "updated_at")?;

        let created_at = DateTime::from_timestamp_millis(created_at_col.value(row_idx))
            .unwrap_or_else(|| {
                warn!(
                    "Invalid created_at timestamp at row {}, using current time",
                    row_idx
                );
                Utc::now()
            });
        let updated_at = DateTime::from_timestamp_millis(updated_at_col.value(row_idx))
            .unwrap_or_else(|| {
                warn!(
                    "Invalid updated_at timestamp at row {}, using current time",
                    row_idx
                );
                Utc::now()
            });

        Ok(Document {
            id: id_col.value(row_idx).to_string(),
            content: content_col.value(row_idx).to_string(),
            source: source_col.value(row_idx).to_string(),
            created_at,
            updated_at,
        })
    }

    /// 向量搜索
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<(f64, Document)>> {
        let req = VectorSearchRequest::builder()
            .query(query)
            .samples(limit as u64)
            .build()
            .context("Failed to build vector search request")?;

        // 获取向量索引的读锁
        let vector_index_opt = self.vector_index.read().await;
        if let Some(vector_index) = vector_index_opt.as_ref() {
            debug!(
                "Performing vector search with query: '{}', limit: {}",
                query, limit
            );

            // 直接使用向量索引的top_n方法，避免二次查询
            let results: Vec<(f64, String, serde_json::Value)> =
                VectorStoreIndex::top_n(vector_index, req)
                    .await
                    .context("Vector search failed")?;

            // 将Value转换为Document
            let documents: Vec<(f64, Document)> = results
                .into_iter()
                .filter_map(|(score, _, value)| {
                    serde_json::from_value::<Document>(value)
                        .map_err(|e| {
                            warn!("Failed to deserialize document: {}", e);
                            e
                        })
                        .ok()
                        .map(|doc| (score, doc))
                })
                .collect();

            debug!("Vector search returned {} documents", documents.len());
            Ok(documents)
        } else {
            debug!("Vector index not initialized, returning empty results");
            Ok(Vec::new())
        }
    }

    /// 返回自身用于 RAG 动态上下文
    pub fn get_vector_index(&self) -> Option<&Self> {
        Some(self)
    }

    /// 返回一个合理的默认文档数量
    /// 注意：这是一个同步方法，无法异步查询真实数量，所以返回一个保守的估计
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        // 对于新创建的空表，返回 0
        // 对于已存在的表，返回一个合理的默认值
        // 实际使用中应该调用 count_documents_async() 获取真实数量
        1 // 返回最小值，确保 RAG 能正常工作但不会误导用户
    }

    /// 异步获取真实的文档数量
    pub async fn count_documents_async(&self) -> Result<usize> {
        let db = lancedb::connect(&self.db_path)
            .execute()
            .await
            .context("Failed to connect to LanceDB for counting")?;

        let table_exists = db
            .table_names()
            .execute()
            .await
            .context("Failed to list table names for counting")?
            .contains(&self.table_name);

        if !table_exists {
            debug!(
                "Table '{}' does not exist, returning 0 documents",
                self.table_name
            );
            return Ok(0);
        }

        let table = db
            .open_table(&self.table_name)
            .execute()
            .await
            .context("Failed to open table for counting")?;

        match table.count_rows(None).await {
            Ok(count) => {
                debug!("Table '{}' has {} documents", self.table_name, count);
                Ok(count)
            },
            Err(e) => {
                warn!(
                    "Failed to count rows in table '{}': {}, returning 0",
                    self.table_name, e
                );
                Ok(0)
            },
        }
    }

    /// 添加文档并生成 embeddings
    pub async fn add_documents_with_embeddings(
        &self, documents: Vec<Document>, embedding_model: M,
    ) -> Result<()>
    where
        M: Clone + Send + Sync + 'static,
    {
        if documents.is_empty() {
            debug!("No documents to add, skipping");
            return Ok(());
        }

        info!(
            "Adding {} documents to table '{}'",
            documents.len(),
            self.table_name
        );

        // 构建 embeddings
        let embeddings = EmbeddingsBuilder::new(embedding_model.clone())
            .documents(documents.clone())
            .context("Failed to create embeddings builder")?
            .build()
            .await
            .context("Failed to build embeddings")?;

        // 维度
        let actual_dims = if let Some((_, emb)) = embeddings.first() {
            emb.first().vec.len()
        } else {
            embedding_model.ndims()
        };

        debug!("Using embedding dimensions: {}", actual_dims);

        // 记录批
        let record_batch = Self::as_record_batch(embeddings, actual_dims)
            .context("Failed to create record batch")?;
        let schema = Self::create_schema(actual_dims);

        // 打开数据库
        let db = lancedb::connect(&self.db_path)
            .execute()
            .await
            .context("Failed to connect to LanceDB for adding documents")?;

        let table_exists = db
            .table_names()
            .execute()
            .await
            .context("Failed to list table names")?
            .contains(&self.table_name);

        let table = if table_exists {
            let table = db
                .open_table(&self.table_name)
                .execute()
                .await
                .context("Failed to open existing table")?;
            let batch_reader = RecordBatchIterator::new(vec![Ok(record_batch)], Arc::new(schema));
            table
                .add(batch_reader)
                .execute()
                .await
                .context("Failed to add documents to existing table")?;
            table
        } else {
            info!("Creating new table '{}'", self.table_name);
            db.create_table(
                &self.table_name,
                RecordBatchIterator::new(vec![Ok(record_batch)], Arc::new(schema)),
            )
            .execute()
            .await
            .context("Failed to create new table")?
        };

        // 重建向量索引
        let search_params = SearchParams::default().column("embedding");
        let new_index = LanceDbVectorIndex::new(table, embedding_model, "id", search_params)
            .await
            .context("Failed to create new vector index")?;

        *self.vector_index.write().await = Some(new_index);
        info!(
            "Successfully added {} documents and rebuilt vector index",
            documents.len()
        );
        Ok(())
    }

    /// 根据ID获取文档
    pub async fn get_document(&self, id: &str) -> Result<Option<Document>> {
        let db = lancedb::connect(&self.db_path)
            .execute()
            .await
            .context("Failed to connect to LanceDB for getting document")?;

        let table_exists = db
            .table_names()
            .execute()
            .await
            .context("Failed to list table names")?
            .contains(&self.table_name);

        if !table_exists {
            debug!(
                "Table '{}' does not exist, document not found",
                self.table_name
            );
            return Ok(None);
        }

        let table = db
            .open_table(&self.table_name)
            .execute()
            .await
            .context("Failed to open table for getting document")?;

        match table
            .query()
            .only_if(format!("id = '{}'", id))
            .limit(1)
            .execute()
            .await
        {
            Ok(mut stream) => {
                if let Ok(Some(batch)) = stream.try_next().await {
                    if batch.num_rows() == 0 {
                        debug!("Document with id '{}' not found", id);
                        return Ok(None);
                    }
                    return Ok(Some(Self::parse_document_from_batch(&batch, 0)?));
                }
                debug!("No batch returned for document id '{}'", id);
                Ok(None)
            },
            Err(e) => {
                warn!("Failed to query document with id '{}': {}", id, e);
                Ok(None)
            },
        }
    }

    /// 分页获取文档
    pub async fn list_documents_paginated(
        &self, limit: usize, offset: usize,
    ) -> Result<(Vec<Document>, usize)> {
        let db = lancedb::connect(&self.db_path)
            .execute()
            .await
            .context("Failed to connect to LanceDB for listing documents")?;

        let table_exists = db
            .table_names()
            .execute()
            .await
            .context("Failed to list table names")?
            .contains(&self.table_name);

        if !table_exists {
            debug!(
                "Table '{}' does not exist, returning empty list",
                self.table_name
            );
            return Ok((Vec::new(), 0));
        }

        let table = db
            .open_table(&self.table_name)
            .execute()
            .await
            .context("Failed to open table for listing documents")?;

        let total = table.count_rows(None).await.unwrap_or(0) as usize;
        let safe_limit = limit.clamp(1, 1000);
        let upto = offset.saturating_add(safe_limit);

        debug!(
            "Listing documents: limit={}, offset={}, total={}",
            safe_limit, offset, total
        );

        match table.query().limit(upto).execute().await {
            Ok(stream) => {
                let batches: Vec<RecordBatch> = stream
                    .try_collect()
                    .await
                    .context("Failed to collect record batches")?;
                let mut documents = Vec::new();

                for batch in batches {
                    if batch.num_rows() == 0 {
                        continue;
                    }

                    for row_idx in 0..batch.num_rows() {
                        match Self::parse_document_from_batch(&batch, row_idx) {
                            Ok(doc) => documents.push(doc),
                            Err(e) => {
                                warn!("Failed to parse document at row {}: {}", row_idx, e);
                                continue;
                            },
                        }
                    }
                }

                documents.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
                let start = offset.min(documents.len());
                let end = (start + safe_limit).min(documents.len());

                let result_docs = documents[start..end].to_vec();
                debug!(
                    "Returning {} documents (requested: limit={}, offset={})",
                    result_docs.len(),
                    safe_limit,
                    offset
                );
                Ok((result_docs, total))
            },
            Err(e) => Err(anyhow::anyhow!("Failed to query documents: {}", e)),
        }
    }

    /// 删除文档
    pub async fn delete_document(&self, id: &str) -> Result<()> {
        let db = lancedb::connect(&self.db_path)
            .execute()
            .await
            .context("Failed to connect to LanceDB for deleting document")?;

        let table = db
            .open_table(&self.table_name)
            .execute()
            .await
            .context("Failed to open table for deleting document")?;

        table
            .delete(&format!("id = '{}'", id))
            .await
            .context("Failed to delete document")?;

        info!("Successfully deleted document with id '{}'", id);
        Ok(())
    }

    /// 重置表（删除现有表）
    pub async fn reset_table(&self) -> Result<()> {
        let db = lancedb::connect(&self.db_path)
            .execute()
            .await
            .context("Failed to connect to LanceDB for resetting table")?;

        let table_exists = db
            .table_names()
            .execute()
            .await
            .context("Failed to list table names")?
            .contains(&self.table_name);

        if table_exists {
            db.drop_table(&self.table_name)
                .await
                .context("Failed to drop table")?;
            info!("🗑️ Dropped table '{}'", self.table_name);
        } else {
            debug!(
                "Table '{}' does not exist, nothing to reset",
                self.table_name
            );
        }

        // 清空向量索引
        *self.vector_index.write().await = None;
        Ok(())
    }

    /// 创建schema
    fn create_schema(dims: usize) -> Schema {
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

    /// 将文档和embeddings转换为RecordBatch
    fn as_record_batch(
        records: Vec<(Document, OneOrMany<Embedding>)>, dims: usize,
    ) -> Result<RecordBatch> {
        if records.is_empty() {
            return Err(anyhow::anyhow!(
                "Cannot create RecordBatch from empty records"
            ));
        }

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

        // 获取实际的embedding维度
        let actual_dims = if let Some((_, emb)) = records.first() {
            emb.first().vec.len()
        } else {
            dims
        };

        debug!(
            "Creating RecordBatch with {} records and {} dimensions",
            records.len(),
            actual_dims
        );

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
            ("embedding", Arc::new(embeddings) as ArrayRef),
        ])
        .map_err(|e| anyhow::anyhow!("Failed to create RecordBatch: {}", e))
    }
}

/// 实现 VectorStoreIndex trait 以兼容现有代码
impl<M: EmbeddingModel> VectorStoreIndex for DocumentStore<M> {
    async fn top_n_ids(
        &self, req: VectorSearchRequest,
    ) -> Result<Vec<(f64, String)>, rig::vector_store::VectorStoreError> {
        let vector_index_opt = self.vector_index.read().await;
        if let Some(vector_index) = vector_index_opt.as_ref() {
            vector_index
                .top_n_ids(req)
                .await
                .map_err(|e| rig::vector_store::VectorStoreError::DatastoreError(Box::new(e)))
        } else {
            Ok(Vec::new())
        }
    }

    async fn top_n<T: for<'a> serde::Deserialize<'a> + Send>(
        &self, req: VectorSearchRequest,
    ) -> Result<Vec<(f64, String, T)>, rig::vector_store::VectorStoreError> {
        let vector_index_opt = self.vector_index.read().await;
        if let Some(vector_index) = vector_index_opt.as_ref() {
            vector_index
                .top_n(req)
                .await
                .map_err(|e| rig::vector_store::VectorStoreError::DatastoreError(Box::new(e)))
        } else {
            Ok(Vec::new())
        }
    }
}

// 包装类型，以便动态上下文使用 'static 引用
#[derive(Clone)]
pub struct DocumentStoreWrapper<M: EmbeddingModel>(pub Arc<DocumentStore<M>>);

impl<M: EmbeddingModel> VectorStoreIndex for DocumentStoreWrapper<M> {
    async fn top_n_ids(
        &self, req: VectorSearchRequest,
    ) -> Result<Vec<(f64, String)>, rig::vector_store::VectorStoreError> {
        VectorStoreIndex::top_n_ids(self.0.as_ref(), req).await
    }

    async fn top_n<T: for<'a> serde::Deserialize<'a> + Send>(
        &self, req: VectorSearchRequest,
    ) -> Result<Vec<(f64, String, T)>, rig::vector_store::VectorStoreError> {
        VectorStoreIndex::top_n(self.0.as_ref(), req).await
    }
}
