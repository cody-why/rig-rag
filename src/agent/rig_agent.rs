use std::sync::{Arc, RwLock};

use rig::{
    agent::Agent,
    completion::Chat,
    prelude::CompletionClient,
    providers::openai::{self},
};

use super::{RigAgentBuilder, file_chunk::FileChunk};
use crate::db::DocumentStore;

#[derive(Clone)]
pub struct RigAgent {
    pub agent: Arc<Agent<openai::CompletionModel>>,
    pub context: Arc<RwLock<RigAgentContext>>,
    pub document_store: Option<Arc<DocumentStore>>,
}

pub struct RigAgentContext {
    pub preamble: String,
    pub temperature: f64,
    pub openai_model: String,
    pub client: openai::Client,
    pub embedding_model: openai::EmbeddingModel,
    pub needs_rebuild: bool, // 标记是否需要重建agent
}

impl RigAgent {
    pub fn builder() -> RigAgentBuilder {
        RigAgentBuilder::default()
    }

    pub fn from_env() -> RigAgentBuilder {
        RigAgentBuilder::from_env()
    }

    pub async fn add_documents(&mut self, documents: Vec<FileChunk>) -> anyhow::Result<()> {
        if let Some(ref store) = self.document_store {
            let mut stored_documents = Vec::new();

            for (i, doc) in documents.into_iter().enumerate() {
                tracing::info!("📄 {} ({} chunks)", doc.filename, doc.chunks.len());
                for (chunk_idx, content) in doc.chunks.into_iter().enumerate() {
                    let stored_doc = crate::db::StoredDocument::new(content, doc.filename.clone())
                        .with_id(format!("{}_{}", i, chunk_idx));
                    stored_documents.push(stored_doc);
                }
            }

            if !stored_documents.is_empty() {
                // 获取 embedding model
                let embedding_model = {
                    let context = self.context.read().unwrap();
                    context.embedding_model.clone()
                };

                // 使用正确的方法添加文档并生成 embeddings
                store
                    .add_documents_with_embeddings(stored_documents, embedding_model)
                    .await?;

                // 标记需要重建 agent 以使用新文档
                if let Ok(mut context) = self.context.write() {
                    context.needs_rebuild = true;
                    tracing::info!("✅ Documents added, marked agent for rebuild");
                }
            }
        }

        Ok(())
    }

    /// 同步向量存储 - LanceDB 已经持久化，这里主要同步 preamble
    pub async fn sync_vector_store(&self) -> anyhow::Result<()> {
        // LanceDB 向量存储是持久化的，不需要重新加载文档
        // 这里主要用于同步其他配置
        tracing::info!("✅ LanceDB vector store is persistent - no sync needed");
        Ok(())
    }

    /// 动态聊天 - 使用当前最新的context构建临时agent进行聊天
    pub async fn dynamic_chat(
        &self,
        message: &str,
        history: Vec<rig::completion::Message>,
    ) -> anyhow::Result<String> {
        // 检查是否有文档存储
        let has_documents = if let Some(ref store) = self.document_store {
            let doc_count = store.list_documents().await.unwrap_or_default().len();
            tracing::info!("📚 Document store has {} documents", doc_count);
            doc_count > 0
        } else {
            tracing::warn!("📚 No document store available");
            false
        };

        // 检查是否需要重建agent
        let needs_rebuild = {
            let context = self.context.read().unwrap();
            context.needs_rebuild
        };

        if needs_rebuild {
            tracing::info!("🔄 Rebuilding agent due to configuration changes");
            // 重建agent
            let rebuilt_agent = self.rebuild_with_sync().await?;

            // 重置标志
            {
                let mut context = self.context.write().unwrap();
                context.needs_rebuild = false;
            }

            // 使用重建的agent进行聊天
            tracing::info!(
                "💬 Using rebuilt agent for chat (has_documents: {})",
                has_documents
            );
            let response = rebuilt_agent
                .agent
                .chat(message, history)
                .await
                .map_err(|e| anyhow::anyhow!("Chat error: {}", e))?;
            Ok(response)
        } else {
            // 直接使用预构建的 agent
            tracing::info!(
                "💬 Using existing agent for chat (has_documents: {})",
                has_documents
            );
            let response = self
                .agent
                .chat(message, history)
                .await
                .map_err(|e| anyhow::anyhow!("Chat error: {}", e))?;
            Ok(response)
        }
    }

    /// 重新构建整个RigAgent以应用最新的配置
    pub async fn rebuild_with_sync(&self) -> anyhow::Result<RigAgent> {
        // 同步向量存储
        self.sync_vector_store().await?;

        // 尝试从文件重新加载preamble（如果有的话）
        let updated_preamble = {
            let preamble_path =
                std::env::var("PREAMBLE_FILE").unwrap_or_else(|_| "data/preamble.txt".to_string());

            match tokio::fs::read_to_string(&preamble_path).await {
                Ok(content) => {
                    tracing::info!(
                        "✅ Reloaded preamble from file during rebuild: {}",
                        preamble_path
                    );
                    Some(content)
                }
                Err(_) => {
                    tracing::debug!(
                        "No preamble file found or failed to read, using context preamble"
                    );
                    None
                }
            }
        };

        // 构建新的agent
        if let Ok(mut context) = self.context.write() {
            // 如果从文件成功加载了新的preamble，更新context
            if let Some(new_preamble) = updated_preamble {
                context.preamble = new_preamble;
            }

            let new_agent = if let Some(ref store) = self.document_store {
                // 使用文档存储构建RAG agent
                context.build_with_document_store(store)
            } else {
                // 构建基础agent
                context.build()
            };

            Ok(RigAgent {
                agent: Arc::new(new_agent),
                context: self.context.clone(),
                document_store: self.document_store.clone(),
            })
        } else {
            anyhow::bail!("Failed to write to agent context for rebuilding");
        }
    }
}

impl RigAgentContext {
    pub fn build(&self) -> Agent<openai::CompletionModel> {
        // 创建一个基础的 agent，没有向量存储
        // 向量存储现在由 LanceDB 处理
        self.client
            .completion_model(&self.openai_model)
            .completions_api()
            .into_agent_builder()
            .temperature(self.temperature) // 0.1-0.3 准确性高，0.5-0.7 创造性高
            .preamble(&self.preamble)
            .build()
    }

    /// 构建带有文档存储的RAG agent
    pub fn build_with_document_store(
        &self,
        document_store: &crate::db::DocumentStore,
    ) -> Agent<openai::CompletionModel> {
        if let Some(vector_index) = document_store.get_vector_index() {
            let total_docs = vector_index.len();
            // RAG 检索数量：取前 3 个最相关文档，而不是所有文档
            let top_k = total_docs.clamp(1, 3);
            tracing::info!(
                "✅ Building RAG agent with {} total documents, top_k={}",
                total_docs,
                top_k
            );

            // 创建包装器以避免生命周期问题
            let store_wrapper = crate::db::DocumentStoreWrapper(Arc::new(document_store.clone()));

            self.client
                .completion_model(&self.openai_model)
                .completions_api()
                .into_agent_builder()
                .temperature(self.temperature)
                .preamble(&self.preamble)
                .dynamic_context(top_k, store_wrapper)
                .build()
        } else {
            tracing::warn!("⚠️ Vector index not available, using basic agent");
            self.build()
        }
    }
}
