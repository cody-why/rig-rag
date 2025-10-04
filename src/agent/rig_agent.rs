use std::sync::{Arc, RwLock};

use rig::{
    agent::Agent,
    completion::Chat,
    prelude::CompletionClient,
    providers::openai::{self},
};

use super::RigAgentBuilder;
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
    pub needs_rebuild: bool,
}

impl RigAgent {
    pub fn builder() -> RigAgentBuilder {
        RigAgentBuilder::default()
    }

    pub fn from_env() -> RigAgentBuilder {
        RigAgentBuilder::from_env()
    }

    // 简化模式：禁用运行时添加文档

    /// 同步向量存储 - LanceDB 已经持久化，这里主要同步 preamble
    pub async fn sync_vector_store(&self) -> anyhow::Result<()> {
        // LanceDB 向量存储是持久化的，不需要重新加载文档
        // 这里主要用于同步其他配置
        tracing::info!("✅ LanceDB vector store is persistent");
        Ok(())
    }

    /// 动态聊天 - 使用当前最新的context构建临时agent进行聊天
    pub async fn dynamic_chat(
        &self,
        message: &str,
        history: Vec<rig::completion::Message>,
    ) -> anyhow::Result<String> {
        // 简化：只要存在向量存储即认为可用
        let has_documents = self.document_store.is_some();

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
        let new_agent = if let Some(ref store) = self.document_store {
            // 创建新的context用于构建agent
            let context = {
                let current_context = self.context.read().unwrap();
                RigAgentContext {
                    client: current_context.client.clone(),
                    embedding_model: current_context.embedding_model.clone(),
                    preamble: updated_preamble
                        .clone()
                        .unwrap_or_else(|| current_context.preamble.clone()),
                    temperature: current_context.temperature,
                    openai_model: current_context.openai_model.clone(),
                    needs_rebuild: false,
                }
            };

            // 使用文档存储构建RAG agent
            context.build_with_document_store(store).await
        } else {
            // 创建新的context用于构建基础agent
            let context = {
                let current_context = self.context.read().unwrap();
                RigAgentContext {
                    client: current_context.client.clone(),
                    embedding_model: current_context.embedding_model.clone(),
                    preamble: updated_preamble
                        .clone()
                        .unwrap_or_else(|| current_context.preamble.clone()),
                    temperature: current_context.temperature,
                    openai_model: current_context.openai_model.clone(),
                    needs_rebuild: false,
                }
            };

            // 构建基础agent
            context.build()
        };

        // 更新原始context的preamble（如果从文件加载了新的）
        if let Some(new_preamble) = updated_preamble
            && let Ok(mut context) = self.context.write()
        {
            context.preamble = new_preamble;
        }

        Ok(RigAgent {
            agent: Arc::new(new_agent),
            context: self.context.clone(),
            document_store: self.document_store.clone(),
        })
    }
}

impl RigAgentContext {
    /// 构建基础 agent
    pub fn build(&self) -> Agent<openai::CompletionModel> {
        self.client
            .completion_model(&self.openai_model)
            .completions_api()
            .into_agent_builder()
            .temperature(self.temperature) // 0.1-0.3 准确性高，0.5-0.7 创造性高
            .preamble(&self.preamble)
            .build()
    }

    /// 计算文档数量 - 提取公共逻辑
    async fn count_documents(&self, document_store: &crate::db::DocumentStore) -> usize {
        if let Some(vector_index) = document_store.get_vector_index() {
            match vector_index.count_documents_async().await {
                Ok(count) => count,
                Err(e) => {
                    tracing::warn!("⚠️ Failed to count documents: {}, using fallback", e);
                    vector_index.len() // 使用同步方法作为后备
                }
            }
        } else {
            0
        }
    }

    /// 计算 top_k 值 - 提取公共逻辑
    fn calculate_top_k(&self, total_docs: usize) -> usize {
        if total_docs == 0 {
            0
        } else if total_docs <= 10 {
            total_docs
        } else {
            total_docs.clamp(1, 10)
        }
    }

    /// 构建带有文档存储的RAG agent
    pub async fn build_with_document_store(
        &self,
        document_store: &crate::db::DocumentStore,
    ) -> Agent<openai::CompletionModel> {
        let total_docs = self.count_documents(document_store).await;

        if total_docs == 0 {
            tracing::info!("📋 No documents found in database, using basic agent");
            return self.build();
        }

        let top_k = self.calculate_top_k(total_docs);
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
    }
}
