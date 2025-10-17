use parking_lot::RwLock;
use rig::{agent::Agent, completion::Chat, prelude::CompletionClient, providers::openai::{self}};
use rig_lancedb::{LanceDbVectorIndex, SearchParams};

use super::RigAgentBuilder;
use crate::config::{AppConfig, LanceDbConfig};

pub struct RigAgent {
    pub agent: RwLock<Agent<openai::CompletionModel>>,
    pub context: RwLock<RigAgentContext>,
}

#[derive(Clone)]
pub struct RigAgentContext {
    pub temperature: f64,
    pub openai_model: String,
    pub client: openai::Client,
    pub embedding_model: openai::EmbeddingModel,
    pub needs_rebuild: bool,
    pub lancedb_config: LanceDbConfig,
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
    pub async fn dynamic_chat(
        &self, message: &str, history: Vec<rig::completion::Message>,
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
        let agent_arc = self.agent.read().clone();
        let response = agent_arc
            .chat(message, history)
            .await
            .map_err(|e| anyhow::anyhow!("Chat error: {}", e))?;
        Ok(response)
    }

    /// 重新构建整个RigAgent以应用最新的配置
    pub async fn rebuild_with_sync(&self) -> anyhow::Result<()> {
        {
            let preamble = load_preamble(&self.context.read().preamble_file);
            self.context.write().preamble = preamble;
        }
        let new_agent = self.build_agent().await?;

        // 替换 agent
        {
            *self.agent.write() = new_agent;
            self.context.write().needs_rebuild = false;
        }

        Ok(())
    }

    /// 从当前context构建agent，避免跨越await持有锁
    async fn build_agent(&self) -> anyhow::Result<Agent<openai::CompletionModel>> {
        // 提取构建agent所需的最小数据
        let (embedding_model, lancedb_config) = {
            let context = self.context.read();
            (
                context.embedding_model.clone(),
                context.lancedb_config.clone(),
            )
        };

        let index = create_vector_index(&lancedb_config, &embedding_model).await?;

        let context = self.context.read();
        let agent = context.build_with_vector_index(index);
        Ok(agent)
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
        &self, vector_index: LanceDbVectorIndex<openai::EmbeddingModel>,
    ) -> Agent<openai::CompletionModel> {
        let top_k = 3; // 可以根据需要调整
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
        let index = create_vector_index(&self.lancedb_config, &self.embedding_model).await?;
        Ok(self.build_with_vector_index(index))
    }

    pub async fn create_vector_index(
        &self,
    ) -> anyhow::Result<LanceDbVectorIndex<openai::EmbeddingModel>> {
        create_vector_index(&self.lancedb_config, &self.embedding_model).await
    }
}

pub async fn create_vector_index(
    lancedb_config: &LanceDbConfig, embedding_model: &openai::EmbeddingModel,
) -> anyhow::Result<LanceDbVectorIndex<openai::EmbeddingModel>> {
    let db = lancedb::connect(&lancedb_config.path).execute().await?;
    let names = db.table_names().execute().await?;
    if !names.contains(&lancedb_config.table_name) {
        anyhow::bail!("LanceDB table '{}' not found", lancedb_config.table_name);
    }
    let table = db.open_table(&lancedb_config.table_name).execute().await?;

    let search_params = SearchParams::default();
    let index =
        LanceDbVectorIndex::new(table, embedding_model.clone(), "id", search_params).await?;

    Ok(index)
}

/// 加载preamble - 从文件加载
pub fn load_preamble(preamble_file: &str) -> String {
    let preamble = "You are a helpful AI assistant.".to_string();
    match std::fs::read_to_string(preamble_file) {
        Ok(content) => {
            tracing::info!("✅ Loaded preamble from file: {}", preamble_file);
            content
        },
        Err(e) => {
            tracing::warn!(
                "⚠️ Failed to read preamble file {}: {}, using default",
                preamble_file,
                e
            );
            preamble
        },
    }
}
