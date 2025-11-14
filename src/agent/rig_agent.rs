use std::sync::atomic::{AtomicPtr, Ordering};

use super::RigAgentBuilder;
use crate::{
    config::{AppConfig, QdrantConfig},
    db::{DocumentStore, SerializableQdrantVectorStore},
};
use async_stream::stream;
use futures::StreamExt;
use parking_lot::RwLock;
use rig::{
    agent::{Agent, MultiTurnStreamItem, Text},
    completion::Chat,
    message::Reasoning,
    prelude::CompletionClient,
    providers::openai::{self},
    streaming::{StreamedAssistantContent, StreamingChat},
};

pub struct RigAgent {
    pub agent: AtomicPtr<Agent<openai::CompletionModel>>,
    pub context: RwLock<RigAgentContext>,
}

// 显式实现 Send，因为 AtomicPtr 和 RwLock 都是 Send 的
unsafe impl Send for RigAgent {}
unsafe impl Sync for RigAgent {}

#[derive(Clone)]
pub struct RigAgentContext {
    pub temperature: f64,
    pub openai_model: String,
    pub client: openai::Client,
    pub embedding_model: openai::EmbeddingModel,
    pub needs_rebuild: bool,
    pub qdrant_config: QdrantConfig,
    pub preamble_file: String,
    pub preamble: String,
}

impl RigAgent {
    /// 从配置创建新的 RigAgent
    pub async fn new_from_config(config: &AppConfig) -> anyhow::Result<RigAgent> {
        let builder = RigAgentBuilder::from_config(config.clone());
        builder.build().await
    }

    /// 动态聊天 - 使用当前最新的context构建临时agent进行聊天
    pub async fn chat(
        &self,
        message: &str,
        history: Vec<rig::completion::Message>,
    ) -> anyhow::Result<String> {
        // 检查是否需要重建agent
        let needs_rebuild = {
            let context = self.context.read();
            context.needs_rebuild
        };

        if needs_rebuild {
            tracing::info!("🔄 Agent needs rebuild, rebuilding with latest documents...");
            // 重建agent以使用最新的文档
            self.rebuild_with_sync().await?;
        }

        // 使用当前（可能已重建）的agent进行聊天
        let agent_ptr = self.agent.load(Ordering::Acquire);
        if agent_ptr.is_null() {
            return Err(anyhow::anyhow!("Agent not initialized"));
        }

        // 安全地解引用原子指针
        let agent = unsafe { &*agent_ptr };
        let response = agent
            .chat(message, history)
            .await
            .map_err(|e| anyhow::anyhow!("Chat error: {}", e))?;
        Ok(response)
    }

    /// 动态流式聊天 - 使用当前最新的context构建临时agent进行流式聊天
    pub async fn stream_chat(
        &self,
        message: &str,
        history: Vec<rig::completion::Message>,
    ) -> anyhow::Result<impl futures::Stream<Item = String> + Unpin> {
        // 检查是否需要重建agent
        let needs_rebuild = {
            let context = self.context.read();
            context.needs_rebuild
        };

        if needs_rebuild {
            tracing::info!("🔄 Agent needs rebuild, rebuilding with latest documents...");
            // 重建agent以使用最新的文档
            self.rebuild_with_sync().await?;
        }

        // 使用当前（可能已重建）的agent进行流式聊天
        let agent_ptr = self.agent.load(Ordering::Acquire);
        if agent_ptr.is_null() {
            return Err(anyhow::anyhow!("Agent not initialized"));
        }

        // 安全地解引用原子指针
        let agent = unsafe { &*agent_ptr };
        let stream_request = agent.stream_chat(message, history);

        // 创建一个简化的流，将复杂的流式响应转换为简单的字符串流
        let stream = Box::pin(stream! {
            let mut stream = stream_request.await;
            while let Some(content) = stream.next().await {
                match content {
                    Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(Text {
                        text,
                    }))) => {
                        yield text;
                    },
                    Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Reasoning(
                        Reasoning { reasoning, .. },
                    ))) => {
                        // yield reasoning.join("\n");
                        tracing::debug!("Reasoning: {:?}", reasoning);
                        yield "Reasoning... Please wait...".to_string();
                    },
                    Ok(MultiTurnStreamItem::FinalResponse(res)) => {
                        tracing::debug!("{:?}", res);
                    },
                    Err(e) => {
                        yield format!("Error: {}", e);
                        break;
                    },
                    _ => {},
                }
            }
        });

        Ok(stream)
    }

    /// 重新构建整个RigAgent以应用最新的配置
    pub async fn rebuild_with_sync(&self) -> anyhow::Result<()> {
        {
            let preamble = load_preamble(&self.context.read().preamble_file);
            self.context.write().preamble = preamble;
        }
        let new_agent = self.build_agent().await?;

        // 替换 agent - 使用原子指针替换
        let new_agent_box = Box::new(new_agent);
        let new_agent_ptr = Box::into_raw(new_agent_box);

        // 原子地替换指针
        let old_agent_ptr = self.agent.swap(new_agent_ptr, Ordering::AcqRel);

        // 清理旧的 agent（如果存在）
        if !old_agent_ptr.is_null() {
            let _ = unsafe { Box::from_raw(old_agent_ptr) };
        }

        self.context.write().needs_rebuild = false;
        Ok(())
    }

    /// 从当前context构建agent，避免跨越await持有锁
    async fn build_agent(&self) -> anyhow::Result<Agent<openai::CompletionModel>> {
        // 提取构建agent所需的最小数据
        let (embedding_model, qdrant_config) = {
            let context = self.context.read();
            (
                context.embedding_model.clone(),
                context.qdrant_config.clone(),
            )
        };

        let index = create_vector_index(&qdrant_config, &embedding_model).await?;
        let context = self.context.read();
        let agent = context.build_with_vector_index(index.0, index.1);
        Ok(agent)
    }

    pub async fn set_needs_rebuild(&self, needs_rebuild: bool) {
        self.context.write().needs_rebuild = needs_rebuild;
    }
}

impl Drop for RigAgent {
    fn drop(&mut self) {
        // 清理原子指针中的 agent
        let agent_ptr = self.agent.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !agent_ptr.is_null() {
            let _ = unsafe { Box::from_raw(agent_ptr) };
        }
    }
}

impl RigAgentContext {
    /// 构建基础 agent
    pub fn build_basic(&self) -> Agent<openai::CompletionModel> {
        self.client
            .completion_model(&self.openai_model)
            .completions_api()
            .into_agent_builder()
            .temperature(self.temperature) // 0.1-0.3 准确性高，0.5-0.7 创造性高
            .preamble(&self.preamble)
            .build()
    }

    /// 构建带有向量索引的RAG agent
    pub fn build_with_vector_index(
        &self,
        vector_index: SerializableQdrantVectorStore<openai::EmbeddingModel>,
        top_k: usize,
    ) -> Agent<openai::CompletionModel> {
        let top_k = top_k.max(1);
        tracing::info!("✅ Building RAG agent with vector index, top_k={}", top_k);
        self.client
            .completion_model(&self.openai_model)
            .completions_api()
            .into_agent_builder()
            .temperature(self.temperature)
            .preamble(&self.preamble)
            .dynamic_context(top_k, vector_index)
            .build()
    }

    /// 构建带有向量索引的RAG agent
    pub async fn build(&self) -> anyhow::Result<Agent<openai::CompletionModel>> {
        let index = create_vector_index(&self.qdrant_config, &self.embedding_model).await?;
        Ok(self.build_with_vector_index(index.0, index.1))
    }

    pub async fn create_vector_index(
        &self,
    ) -> anyhow::Result<(SerializableQdrantVectorStore<openai::EmbeddingModel>, usize)> {
        create_vector_index(&self.qdrant_config, &self.embedding_model).await
    }
}

pub async fn create_vector_index(
    qdrant_config: &QdrantConfig,
    embedding_model: &openai::EmbeddingModel,
) -> anyhow::Result<(SerializableQdrantVectorStore<openai::EmbeddingModel>, usize)> {
    let store: DocumentStore = DocumentStore::with_config(qdrant_config);
    store.create_vector_index(embedding_model.clone()).await
}

/// 加载preamble - 从文件加载
pub fn load_preamble(preamble_file: &str) -> String {
    let preamble = "You are a helpful AI assistant.".to_string();
    match std::fs::read_to_string(preamble_file) {
        Ok(content) => {
            tracing::info!("✅ Loaded preamble from file: {}", preamble_file);
            content
        }
        Err(e) => {
            tracing::warn!(
                "⚠️ Failed to read preamble file {}: {}, using default",
                preamble_file,
                e
            );
            preamble
        }
    }
}
