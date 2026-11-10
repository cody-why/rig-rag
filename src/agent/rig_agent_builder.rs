use std::sync::atomic::AtomicPtr;

use parking_lot::RwLock;
use rig::providers::openai::{OpenAI, OpenAIConfig};
use tracing::info;

use super::rig_agent::RigAgent;
use crate::{
    agent::rig_agent::{RigAgentContext, load_preamble},
    config::AppConfig,
    db::EmbeddingHandle,
};
pub struct RigAgentBuilder {
    config: AppConfig,
}

impl RigAgentBuilder {
    pub fn from_env() -> RigAgentBuilder {
        let config = AppConfig::from_env();
        Self::from_config(config)
    }

    pub fn from_config(config: AppConfig) -> RigAgentBuilder {
        RigAgentBuilder { config }
    }

    /// 获取配置的引用
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// 获取配置的可变引用
    pub fn config_mut(&mut self) -> &mut AppConfig {
        &mut self.config
    }

    /// 构建agent
    pub async fn build(self) -> anyhow::Result<RigAgent> {
        info!("🚀 Initializing RigAgent...");

        // 初始化OpenAI客户端
        let client = self.init_openai_client();

        // 初始化Embedding客户端
        let embedding_model = self.init_embedding_client();
        info!(
            model = %self.config.embedding_model,
            "Initialized embedding model"
        );

        // 创建上下文和代理
        let context = RigAgentContext {
            client: client.clone(),
            embedding_model,
            temperature: self.config.temperature,
            openai_model: self.config.openai_model.clone(),
            qdrant_config: self.config.qdrant.clone(),
            preamble_file: self.config.preamble_file.clone(),
            needs_rebuild: false,
            preamble: load_preamble(&self.config.preamble_file),
            documents_dir: self.config.documents_dir.clone(),
            documents_exclude_dirs: self.config.documents_exclude_dirs.clone(),
            rag_top_k: self.config.rag_top_k,
        };

        let rag_agent = match context.build().await {
            Ok(agent) => {
                info!("ℹ️ Building RAG agent with vector index");
                agent
            }
            Err(e) => {
                info!("ℹ️ No vector index available ({}), using basic agent", e);
                context.build_basic()
            }
        };

        info!("✅ RigAgent initialized successfully");

        // 将 agent 包装在 Box 中并转换为原始指针
        let agent_box = Box::new(rag_agent);
        let agent_ptr = Box::into_raw(agent_box);
        let agent_atomic = AtomicPtr::new(agent_ptr);

        Ok(RigAgent {
            agent: agent_atomic,
            context: RwLock::new(context),
        })
    }

    /// 初始化OpenAI客户端
    fn init_openai_client(&self) -> OpenAI {
        OpenAIConfig::new(&self.config.openai_api_key)
            .with_base_url(&self.config.openai_base_url)
            .client()
    }

    fn init_embedding_client(&self) -> EmbeddingHandle {
        let embedding_client = OpenAIConfig::new(&self.config.embedding_api_key)
            .with_base_url(&self.config.embedding_url)
            .client();

        embedding_client
            .embedding(&self.config.embedding_model, None)
            .erase()
    }
}
