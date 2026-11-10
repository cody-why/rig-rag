use std::sync::atomic::{AtomicPtr, Ordering};

use super::RigAgentBuilder;
use crate::{
    config::{AppConfig, QdrantConfig},
    db::{DocumentStore, EmbeddingHandle, SerializableQdrantVectorStore},
};
use async_stream::stream;
use futures::StreamExt;
use parking_lot::RwLock;
use rig::{
    Agent, AgentBuilder,
    agent::MultiTurnStreamItem,
    completion::Message,
    message::AssistantContent,
    providers::openai::OpenAI,
    streaming::{Item, StreamEvent},
};

pub struct RigAgent {
    pub agent: AtomicPtr<Agent>,
    pub context: RwLock<RigAgentContext>,
}

// 显式实现 Send，因为 AtomicPtr 和 RwLock 都是 Send 的
unsafe impl Send for RigAgent {}
unsafe impl Sync for RigAgent {}

#[derive(Clone)]
pub struct RigAgentContext {
    pub temperature: f64,
    pub openai_model: String,
    pub client: OpenAI,
    pub embedding_model: EmbeddingHandle,
    pub needs_rebuild: bool,
    pub qdrant_config: QdrantConfig,
    pub preamble_file: String,
    pub preamble: String,
    /// 本地 markdown 文档目录（检索时一并纳入）
    pub documents_dir: String,
    /// 同步时排除的子目录
    pub documents_exclude_dirs: Vec<String>,
    /// RAG 检索 top_k
    pub rag_top_k: usize,
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
        history: Vec<Message>,
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
            .prompt(message)
            .history(history)
            .await
            .map_err(|e| anyhow::anyhow!("Chat error: {}", e))?;
        Ok(response.output())
    }

    /// 动态流式聊天 - 使用当前最新的context构建临时agent进行流式聊天
    pub async fn stream_chat(
        &self,
        message: &str,
        history: Vec<Message>,
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
        let mut stream_request = agent.prompt(message).history(history).stream();

        // 创建一个简化的流，将复杂的流式响应转换为简单的字符串流
        let stream = Box::pin(stream! {
            let mut saw_text_delta = false;
            let mut text_deltas = 0usize;

            while let Some(content) = stream_request.next().await {
                match content {
                    Ok(MultiTurnStreamItem::StreamAssistantItem(Item::Event(StreamEvent::Text {
                        text,
                        ..
                    }))) => {
                        if !text.is_empty() {
                            saw_text_delta = true;
                            text_deltas += 1;
                            yield text;
                        }
                    },
                    Ok(MultiTurnStreamItem::StreamAssistantItem(Item::Event(StreamEvent::End {
                        content,
                        ..
                    }))) => {
                        // 部分兼容网关不推 delta，只在 End 给全文
                        if !saw_text_delta
                            && let AssistantContent::Text(text) = content
                            && !text.text.is_empty()
                        {
                            tracing::debug!("Stream End without deltas, yielding full text");
                            yield text.text;
                        }
                    },
                    Ok(MultiTurnStreamItem::StreamAssistantItem(Item::Event(StreamEvent::Reasoning {
                        ..
                    }))) => {
                        tracing::debug!("Reasoning stream event");
                    },
                    Ok(MultiTurnStreamItem::FinalResponse(res)) => {
                        if !saw_text_delta {
                            let out = res.output();
                            if !out.is_empty() {
                                tracing::debug!("FinalResponse without deltas, yielding full text");
                                yield out;
                            }
                        }
                        tracing::debug!(
                            text_deltas,
                            usage = ?res.usage(),
                            "Stream completed"
                        );
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

        // 重建前同步本地 markdown 目录，保证检索包含最新文件
        {
            let (documents_dir, exclude_dirs, embedding_model, qdrant_config) = {
                let ctx = self.context.read();
                (
                    ctx.documents_dir.clone(),
                    ctx.documents_exclude_dirs.clone(),
                    ctx.embedding_model.clone(),
                    ctx.qdrant_config.clone(),
                )
            };
            let store = DocumentStore::with_config(&qdrant_config);
            if let Err(e) = crate::utils::sync_directory_documents(
                &documents_dir,
                &exclude_dirs,
                &store,
                embedding_model,
            )
            .await
            {
                tracing::warn!("Failed to sync DOCUMENTS_DIR during rebuild: {}", e);
            }
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
    async fn build_agent(&self) -> anyhow::Result<Agent> {
        let (embedding_model, qdrant_config, top_k) = {
            let context = self.context.read();
            (
                context.embedding_model.clone(),
                context.qdrant_config.clone(),
                context.rag_top_k,
            )
        };

        let index = create_vector_index(&qdrant_config, embedding_model).await?;
        let context = self.context.read();
        Ok(context.build_with_vector_index(index, top_k))
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
    pub fn build_basic(&self) -> Agent {
        AgentBuilder::new(self.client.chat(&self.openai_model))
            .temperature(self.temperature)
            .preamble(&self.preamble)
            .build()
    }

    /// 构建带有向量索引的RAG agent
    pub fn build_with_vector_index(
        &self,
        vector_index: SerializableQdrantVectorStore,
        top_k: usize,
    ) -> Agent {
        let top_k = top_k.max(1);
        tracing::info!("✅ Building RAG agent with vector index, top_k={}", top_k);
        AgentBuilder::new(self.client.chat(&self.openai_model))
            .temperature(self.temperature)
            .preamble(&self.preamble)
            .dynamic_context(top_k, vector_index)
            .build()
    }

    /// 构建带有向量索引的RAG agent
    pub async fn build(&self) -> anyhow::Result<Agent> {
        let index = create_vector_index(&self.qdrant_config, self.embedding_model.clone()).await?;
        Ok(self.build_with_vector_index(index, self.rag_top_k))
    }

    pub async fn create_vector_index(&self) -> anyhow::Result<SerializableQdrantVectorStore> {
        create_vector_index(&self.qdrant_config, self.embedding_model.clone()).await
    }
}

pub async fn create_vector_index(
    qdrant_config: &QdrantConfig,
    embedding_model: EmbeddingHandle,
) -> anyhow::Result<SerializableQdrantVectorStore> {
    let store = DocumentStore::with_config(qdrant_config);
    store.create_vector_index(embedding_model).await
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
