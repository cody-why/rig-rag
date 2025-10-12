use std::sync::{Arc, RwLock};

use anyhow::Context;
use rig::prelude::EmbeddingsClient;
use tracing::{debug, info, warn};

use super::rig_agent::RigAgent;
use crate::{agent::rig_agent::RigAgentContext, db::DocumentStore, utils::get_env_or_panic};

#[derive(Default)]
pub struct RigAgentBuilder {
    preamble_file: String,
    preamble: String,
    temperature: f64,
    documents_dir: String,
    openai_api_key: String,
    openai_base_url: String,
    openai_model: String,
    embedding_api_key: String,
    embedding_url: String,
    embedding_model: String,
    lancedb_path: String,
    document_store: Option<Arc<DocumentStore>>,
}

impl RigAgentBuilder {
    pub fn from_env() -> RigAgentBuilder {
        let preamble_file = get_env_or_panic("PREAMBLE_FILE");
        let temperature = get_env_or_panic("TEMPERATURE")
            .parse::<f64>()
            .expect("Failed to parse temperature");
        let documents_dir = get_env_or_panic("DOCUMENTS_DIR");
        let lancedb_path = get_env_or_panic("LANCEDB_PATH");

        let openai_api_key = get_env_or_panic("OPENAI_API_KEY");
        let openai_base_url = get_env_or_panic("OPENAI_BASE_URL");
        let openai_model = get_env_or_panic("OPENAI_MODEL");

        let embedding_api_key = get_env_or_panic("EMBEDDING_API_KEY");
        let embedding_url = get_env_or_panic("EMBEDDING_BASE_URL");
        let embedding_model = get_env_or_panic("EMBEDDING_MODEL");

        let (preamble, preamble_file) = if preamble_file.is_empty() {
            // 如果没有设置preamble文件，使用默认preamble
            (
                "You are a helpful AI assistant.".to_string(),
                "".to_string(),
            )
        } else {
            let _preamble_file = std::env::current_dir().unwrap().join(&preamble_file);
            info!("preamble_file: {}", _preamble_file.display());
            let preamble = std::fs::read_to_string(&_preamble_file)
                .unwrap_or_else(|_| "You are a helpful AI assistant.".to_string());
            let preamble_file = _preamble_file
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            (preamble, preamble_file)
        };

        RigAgentBuilder {
            preamble,
            preamble_file,
            temperature,
            documents_dir,
            openai_api_key,
            openai_base_url,
            openai_model,
            embedding_api_key,
            embedding_url,
            embedding_model,
            lancedb_path,
            document_store: None,
        }
    }

    /// 设置预设
    pub fn preamble(&mut self, preamble: &str) -> &mut Self {
        self.preamble = preamble.to_string();
        self
    }

    /// 设置温度 0.1-0.3 准确性高，0.5-0.7 创造性高
    pub fn temperature(&mut self, temperature: f64) -> &mut Self {
        self.temperature = temperature;
        self
    }

    /// 设置文档目录
    pub fn documents_dir(&mut self, documents_dir: &str) -> &mut Self {
        self.documents_dir = documents_dir.to_string();
        self
    }

    /// 设置openai api key
    pub fn openai_api_key(&mut self, openai_api_key: &str) -> &mut Self {
        self.openai_api_key = openai_api_key.to_string();
        self
    }

    /// 设置openai base url
    pub fn openai_base_url(&mut self, openai_base_url: &str) -> &mut Self {
        self.openai_base_url = openai_base_url.to_string();
        self
    }

    /// 设置openai model
    pub fn openai_model(&mut self, openai_model: &str) -> &mut Self {
        self.openai_model = openai_model.to_string();
        self
    }

    /// 设置embedding api key
    pub fn embedding_api_key(&mut self, embedding_api_key: &str) -> &mut Self {
        self.embedding_api_key = embedding_api_key.to_string();
        self
    }

    /// 设置embedding url
    pub fn embedding_url(&mut self, embedding_url: &str) -> &mut Self {
        self.embedding_url = embedding_url.to_string();
        self
    }

    /// 设置embedding model
    pub fn embedding_model(&mut self, embedding_model: &str) -> &mut Self {
        self.embedding_model = embedding_model.to_string();
        self
    }

    /// 设置文档存储
    pub fn document_store(&mut self, document_store: Arc<DocumentStore>) -> &mut Self {
        self.document_store = Some(document_store);
        self
    }

    /// 构建agent
    pub async fn build(self) -> anyhow::Result<RigAgent> {
        info!("🚀 Initializing RigAgent...");

        // 并发初始化客户端
        let (client, embedding_model) = self
            .init_clients()
            .await
            .context("Failed to initialize clients")?;

        // 加载preamble
        let final_preamble = self
            .load_preamble()
            .await
            .context("Failed to load preamble")?;

        // 创建 LanceDB 存储
        let table_name = "documents";
        let document_store = self
            .initialize_document_store(table_name, embedding_model.clone())
            .await;

        // 创建上下文和代理
        let context = RigAgentContext {
            client: client.clone(),
            embedding_model: embedding_model.clone(),
            preamble: final_preamble.clone(),
            temperature: self.temperature,
            openai_model: self.openai_model.clone(),
            needs_rebuild: false, // 初始化时不需要重建
        };

        let rag_agent = self
            .build_rag_agent(&client, &final_preamble, document_store.as_ref())
            .await?;

        info!("✅ RigAgent initialized successfully");

        Ok(RigAgent {
            agent: Arc::new(rag_agent),
            context: Arc::new(RwLock::new(context)),
            document_store,
        })
    }

    /// 初始化OpenAI客户端
    async fn init_clients(
        &self,
    ) -> anyhow::Result<(
        rig::providers::openai::Client,
        rig::providers::openai::EmbeddingModel,
    )> {
        use rig::providers::openai::Client;

        debug!("Initializing OpenAI clients...");

        let client = Client::builder(&self.openai_api_key)
            .base_url(&self.openai_base_url)
            .build()
            .context("Failed to create OpenAI completion client")?;

        let embedding_client = Client::builder(&self.embedding_api_key)
            .base_url(&self.embedding_url)
            .build()
            .context("Failed to create OpenAI embedding client")?;

        let model = embedding_client.embedding_model(&self.embedding_model);

        debug!("OpenAI clients initialized successfully");
        Ok((client, model))
    }

    /// 加载preamble - 从文件加载
    async fn load_preamble(&self) -> anyhow::Result<String> {
        // 如果设置了preamble文件，优先从文件读取最新内容
        if !self.preamble_file.is_empty() {
            let preamble_path =
                std::env::var("PREAMBLE_FILE").unwrap_or_else(|_| "data/preamble.txt".to_string());

            match tokio::fs::read_to_string(&preamble_path).await {
                Ok(content) => {
                    info!("✅ Loaded preamble from file: {}", preamble_path);
                    Ok(content)
                },
                Err(e) => {
                    warn!(
                        "⚠️ Failed to read preamble file {}: {}, using default",
                        preamble_path, e
                    );
                    Ok(self.preamble.clone())
                },
            }
        } else {
            // 没有设置preamble文件，使用初始化时的preamble
            debug!("Using default preamble (no file specified)");
            Ok(self.preamble.clone())
        }
    }

    /// 初始化文档存储
    async fn initialize_document_store(
        &self, table_name: &str, embedding_model: rig::providers::openai::EmbeddingModel,
    ) -> Option<Arc<DocumentStore>> {
        if let Some(store) = &self.document_store {
            debug!("Using provided document store");
            return Some(store.clone());
        }
        let store = DocumentStore::new(&self.lancedb_path, table_name);

        store.load_existing_index(embedding_model).await.unwrap();

        Some(Arc::new(store))
    }

    /// 构建RAG代理 - 使用 RigAgentContext 的公共方法
    async fn build_rag_agent(
        &self, client: &rig::providers::openai::Client, preamble: &str,
        document_store: Option<&Arc<DocumentStore>>,
    ) -> anyhow::Result<rig::agent::Agent<rig::providers::openai::CompletionModel>> {
        let context = RigAgentContext {
            client: client.clone(),
            embedding_model: self.init_clients().await?.1,
            preamble: preamble.to_string(),
            temperature: self.temperature,
            openai_model: self.openai_model.clone(),
            needs_rebuild: false,
        };

        match document_store {
            Some(store) => {
                info!("ℹ️ Building RAG agent with document store");
                Ok(context.build_with_document_store(store.clone()).await)
            },
            None => {
                info!("ℹ️ No document store configured, using basic agent");
                Ok(context.build())
            },
        }
    }
}
