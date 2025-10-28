use std::sync::Arc;

use anyhow::{Context, Result};
use arrow_array::{
    ArrayRef, FixedSizeListArray, RecordBatch, RecordBatchIterator, StringArray,
    TimestampMillisecondArray, types::Float64Type,
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use lancedb::arrow::arrow_schema::{DataType, Field, Fields, Schema, TimeUnit};
use lancedb::index::vector::IvfPqIndexBuilder;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::table::OptimizeAction;
use rig::embeddings::Embedding;
use rig::{
    Embed, OneOrMany,
    embeddings::{EmbeddingModel, EmbeddingsBuilder},
    vector_store::VectorStoreIndex,
    vector_store::request::VectorSearchRequest,
};
use rig_lancedb::{LanceDbVectorIndex, SearchParams};
use serde::{Deserialize, Deserializer, Serialize};
use tracing::{debug, info, warn};

use crate::config::LanceDbConfig;

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
/// 建议使用 Arc<DocumentStore<M>> 来共享实例
pub struct DocumentStore<M: EmbeddingModel> {
    db_path: String,
    table_name: String,
    _phantom: std::marker::PhantomData<M>,
}

impl<M: EmbeddingModel> DocumentStore<M> {
    pub fn new(db_path: &str, table_name: &str) -> Self {
        Self {
            db_path: db_path.to_string(),
            table_name: table_name.to_string(),
            _phantom: std::marker::PhantomData,
        }
    }

    /// 使用 LanceDbConfig 创建 DocumentStore
    pub fn with_config(config: &LanceDbConfig) -> Self {
        Self {
            db_path: config.path.clone(),
            table_name: config.table_name.clone(),
            _phantom: std::marker::PhantomData,
        }
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

    /// 创建向量索引
    pub async fn create_vector_index(&self, embedding_model: M) -> Result<LanceDbVectorIndex<M>>
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
            anyhow::bail!("Table '{}' does not exist", self.table_name);
        }

        let table = db
            .open_table(&self.table_name)
            .execute()
            .await
            .context("Failed to open table")?;

        let search_params = SearchParams::default().column("embedding");
        LanceDbVectorIndex::new(table, embedding_model, "id", search_params)
            .await
            .context("Failed to create vector index")
    }

    /// 向量搜索
    pub async fn search(
        &self,
        vector_index: &LanceDbVectorIndex<M>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(f64, Document)>> {
        let req = VectorSearchRequest::builder()
            .query(query)
            .samples(limit as u64)
            .build()
            .context("Failed to build vector search request")?;

        debug!(
            "Performing vector search with query: '{}', limit: {}",
            query, limit
        );

        // 使用传入的向量索引进行搜索
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
            }
            Err(e) => {
                warn!(
                    "Failed to count rows in table '{}': {}, returning 0",
                    self.table_name, e
                );
                Ok(0)
            }
        }
    }

    /// 添加文档并生成 embeddings
    pub async fn add_documents_with_embeddings(
        &self,
        documents: Vec<Document>,
        embedding_model: M,
    ) -> Result<()>
    where
        M: Clone + Send + Sync + 'static,
    {
        if documents.is_empty() {
            debug!("No documents to add, skipping");
            return Ok(());
        }
        let len = documents.len();
        info!("Adding {} documents to table '{}'", len, self.table_name);

        // 构建 embeddings
        let embeddings = EmbeddingsBuilder::new(embedding_model.clone())
            .documents(documents)
            .context("Failed to create embeddings builder")?
            .build()
            .await
            .context("Failed to build embeddings")?;

        // 维度
        let dims = if let Some((_, emb)) = embeddings.first() {
            emb.first().vec.len()
        } else {
            embedding_model.ndims()
        };

        debug!("Using embedding dimensions: {}", dims);
        // 记录批
        let record_batch =
            Self::as_record_batch(embeddings, dims).context("Failed to create record batch")?;
        let schema = Self::create_schema(dims);
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

        let batch_reader = RecordBatchIterator::new(vec![Ok(record_batch)], Arc::new(schema));

        let table = if table_exists {
            let table = db
                .open_table(&self.table_name)
                .execute()
                .await
                .context("Failed to open existing table")?;
            table
                .add(batch_reader)
                .execute()
                .await
                .context("Failed to add documents to existing table")?;
            table
        } else {
            info!("Creating new table '{}'", self.table_name);

            db.create_table(&self.table_name, batch_reader)
                .execute()
                .await
                .context("Failed to create new table")?
        };

        self.rebuild_index(&table).await?;

        info!(
            "Successfully added {} documents to table '{}'",
            len, self.table_name
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
            }
            Err(e) => {
                warn!("Failed to query document with id '{}': {}", id, e);
                Ok(None)
            }
        }
    }

    /// 分页获取文档
    pub async fn list_documents_paginated(
        &self,
        limit: usize,
        offset: usize,
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
                            }
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
            }
            Err(e) => Err(anyhow::anyhow!("Failed to query documents: {}", e)),
        }
    }

    /// 删除文档
    /// 如果id包含分块标识，删除所有相关的分块文档
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

        // 检查是否是分块文档的base_id
        let query_condition = if id.ends_with("_CHUNKED") {
            // 分块文档：删除所有以base_id开头的文档
            let base_id = id.strip_suffix("_CHUNKED").unwrap_or(id);
            format!("id LIKE '{}%'", base_id)
        } else {
            // 普通文档：精确匹配
            format!("id = '{}'", id)
        };

        table
            .delete(&query_condition)
            .await
            .context("Failed to delete document")?;

        info!(
            "Successfully deleted document(s) with condition: {}",
            query_condition
        );

        // LanceDB 的 delete 操作默认是软删除（标记删除）
        // 需要调用 optimize 来物理删除数据，压缩文件，并重建索引
        info!("🔄 Optimizing table to physically remove deleted documents...");
        let _stats = table
            .optimize(OptimizeAction::All)
            .await
            .context("Failed to optimize table after deletion")?;
        info!("✅ Table optimized, deleted documents physically removed");

        info!("🔄 Document deleted, vector index will be rebuilt by RigAgent when needed");

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
            db.drop_table(&self.table_name, &[])
                .await
                .context("Failed to drop table")?;
            info!("🗑️ Dropped table '{}'", self.table_name);
        }

        info!("🔄 Table reset, vector index will be rebuilt by RigAgent when needed");
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
        records: Vec<(Document, OneOrMany<Embedding>)>,
        dims: usize,
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

        info!(
            "Creating RecordBatch with {} records and {} dimensions",
            records.len(),
            dims
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
            dims as i32,
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

    /// 重建索引
    pub async fn rebuild_index(&self, table: &lancedb::Table) -> Result<()> {
        // See [LanceDB indexing](https://lancedb.github.io/lancedb/concepts/index_ivfpq/#product-quantization) for more information
        if table.index_stats("embedding").await?.is_none() {
            // 检查数据量，IVF-PQ索引需要足够的数据进行训练
            let row_count = table.count_rows(None).await.unwrap_or(0);

            if row_count < 100 {
                info!(
                    "Skipping index creation: only {} rows available, need at least 100 rows for IVF-PQ index",
                    row_count
                );
                return Ok(());
            }

            info!("Creating IVF-PQ index for {} rows", row_count);

            // 根据数据量调整索引参数
            // 对于小数据集，使用较少的分区
            let num_partitions = if row_count < 1000 {
                8.min(row_count as u32 / 2).max(2)
            } else {
                128
            };

            // 设置合适的子向量数量
            let num_sub_vectors = if row_count < 100 { 8 } else { 96 };

            debug!(
                "Creating index with {} partitions and {} sub-vectors for {} rows",
                num_partitions, num_sub_vectors, row_count
            );

            table
                .create_index(
                    &["embedding"],
                    lancedb::index::Index::IvfPq(
                        IvfPqIndexBuilder::default()
                            .num_partitions(num_partitions)
                            .num_sub_vectors(num_sub_vectors),
                    ),
                )
                .execute()
                .await
                .context("Failed to create index")?;

            info!("Successfully created IVF-PQ index");
        } else {
            debug!("Index already exists, skipping creation");
        }
        Ok(())
    }
}
