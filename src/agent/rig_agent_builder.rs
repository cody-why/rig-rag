use std::sync::{Arc, RwLock};

use anyhow::Context;
use rig::{
    embeddings::EmbeddingsBuilder, providers::openai,
    vector_store::in_memory_store::InMemoryVectorStore,
};
use tracing::info;

use crate::{
    agent::{
        file_chunk::FileChunk,
        rig_agent::{Document, RigAgentContext},
    },
    utils::get_env_or_panic,
};

use super::rig_agent::RigAgent;

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
}

impl RigAgentBuilder {
    pub fn from_env() -> RigAgentBuilder {
        let preamble_file = get_env_or_panic("PREAMBLE_FILE");
        let temperature = get_env_or_panic("TEMPERATURE")
            .parse::<f64>()
            .expect("Failed to parse temperature");
        let documents_dir = get_env_or_panic("DOCUMENTS_DIR");

        let openai_api_key = get_env_or_panic("OPENAI_API_KEY");
        let openai_base_url = get_env_or_panic("OPENAI_BASE_URL");
        let openai_model = get_env_or_panic("OPENAI_MODEL");

        let embedding_api_key = get_env_or_panic("EMBEDDING_API_KEY");
        let embedding_url = get_env_or_panic("EMBEDDING_BASE_URL");
        let embedding_model = get_env_or_panic("EMBEDDING_MODEL");

        let _preamble_file = std::env::current_dir().unwrap().join(&preamble_file);
        info!("preamble_file: {}", _preamble_file.display());
        let preamble =
            std::fs::read_to_string(&_preamble_file).expect("Failed to read preamble file");
        let preamble_file = _preamble_file.file_name().unwrap().to_str().unwrap().to_string();

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

    /// 构建agent
    pub async fn build(self) -> anyhow::Result<RigAgent> {
        let client = openai::Client::from_url(&self.openai_api_key, &self.openai_base_url);

        // 加载文件
        let documents_dir = std::env::current_dir()?.join(&self.documents_dir);
        info!("Loading documents from {}", documents_dir.display());

        let chunks = FileChunk::load_files(documents_dir.join("*.*"), &self.preamble_file)
            .context("Failed to load documents")?;

        info!(
            "Successfully loaded and chunked {} document chunks",
            chunks.iter().fold(0, |acc, doc| acc + doc.chunks.len())
        );

        // 创建嵌入模型
        // 英文使用nomic-embed-text, 中文使用bge-m3
        let embedding_client =
            openai::Client::from_url(&self.embedding_api_key, &self.embedding_url);
        let model = embedding_client.embedding_model(&self.embedding_model);

        // 创建嵌入构建器
        let mut builder = EmbeddingsBuilder::new(model.clone());

        // 添加来自 markdown 文档的块
        for (i, doc) in chunks.into_iter().enumerate() {
            println!("{} {} chunks: {}", i + 1, doc.filename, doc.chunks.len());
            for content in doc.chunks {
                builder = builder.document(Document {
                    id: format!("document{}", i + 1),
                    content,
                })?;
            }
        }

        // 构建嵌入
        info!("Generating embeddings...");
        let embeddings = builder.build().await?;
        info!("Successfully generated embeddings");

        // 创建向量存储和索引
        let vector_store = InMemoryVectorStore::from_documents(embeddings);
        let context = RigAgentContext {
            client: client.clone(),
            embedding_model: model.clone(),
            vector_store: vector_store.clone(),
            preamble: self.preamble.clone(),
            temperature: self.temperature,
            openai_model: self.openai_model.clone(),
        };

        let index = vector_store.index(model);
        info!("Successfully created vector store and index");
        let len = index.len();
        // 创建 RAG 代理
        info!("Initializing RAG agent...");
        let rag_agent = client
            .agent(&self.openai_model)
            .temperature(self.temperature) // 0.1-0.3 准确性高，0.5-0.7 创造性高
            .preamble(&self.preamble)
            .dynamic_context(len, index)
            .build();

        Ok(RigAgent {
            agent: Arc::new(rag_agent),
            context: Arc::new(RwLock::new(context)),
        })
    }
}
