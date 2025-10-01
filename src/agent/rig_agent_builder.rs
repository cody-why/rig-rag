use std::sync::{Arc, RwLock};

use anyhow::Context;
use rig::prelude::{CompletionClient, EmbeddingsClient};
use tokio::try_join;
use tracing::info;

use crate::{
    agent::{file_chunk::FileChunk, rig_agent::RigAgentContext},
    db::{DocumentStore, StoredDocument},
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
        let (client, embedding_model) = self.init_clients().await?;

        // 并发加载 preamble 和文档
        let (final_preamble, chunks) = try_join!(self.load_preamble(), self.load_documents())?;

        let total_chunks = chunks.iter().map(|doc| doc.chunks.len()).sum::<usize>();
        info!(
            "📊 Loaded {} documents with {} total chunks",
            chunks.len(),
            total_chunks
        );

        // 详细记录每个文档的信息
        for (idx, doc) in chunks.iter().enumerate() {
            info!(
                "  📄 Document {}: {} ({} chunks)",
                idx + 1,
                doc.filename,
                doc.chunks.len()
            );
        }

        // 创建 LanceDB 存储
        let table_name = "documents";

        let document_store = if self.document_store.is_some() {
            self.document_store
        } else {
            Some(Arc::new(
                DocumentStore::new(&self.lancedb_path, table_name).await?,
            ))
        };

        // 准备文档数据
        let mut documents = Vec::new();
        let mut doc_count = 0;

        for (file_idx, doc) in chunks.into_iter().enumerate() {
            info!("  📄 {} ({} chunks)", doc.filename, doc.chunks.len());

            for (chunk_idx, content) in doc.chunks.into_iter().enumerate() {
                if content.trim().is_empty() {
                    continue;
                }

                let stored_doc = StoredDocument::new(content, doc.filename.clone())
                    .with_id(format!("{}_{}", file_idx, chunk_idx));

                documents.push(stored_doc);
                doc_count += 1;
            }
        }

        // 初始化 LanceDB 向量存储
        let mut vector_store_initialized = false;
        let mut has_documents = false; // 标记是否有文档可以检索

        if let Some(ref store_arc) = document_store {
            info!("📍 LanceDB path: {}", self.lancedb_path);
            info!("🔧 Embedding model: {}", self.embedding_model);

            // 首先尝试加载已存在的向量索引
            info!("🔍 Checking for existing LanceDB data...");
            match store_arc.load_existing_index(embedding_model.clone()).await {
                Ok(true) => {
                    vector_store_initialized = true;
                    has_documents = true;
                    info!("✅ Successfully loaded existing LanceDB vector index");
                }
                Ok(false) => {
                    info!("📋 No existing data found");

                    // 如果有新文档，则初始化
                    if !documents.is_empty() {
                        info!(
                            "🔮 Initializing LanceDB with {} new documents...",
                            doc_count
                        );

                        match store_arc
                            .initialize_with_embeddings(documents, embedding_model.clone())
                            .await
                        {
                            Ok(()) => {
                                vector_store_initialized = true;
                                has_documents = true;
                                info!(
                                    "✅ Successfully initialized LanceDB with {} documents",
                                    doc_count
                                );
                            }
                            Err(e) => {
                                tracing::error!("❌ Failed to initialize LanceDB: {}", e);
                                tracing::error!("🔍 Error details: {:?}", e);
                                tracing::warn!(
                                    "🔄 Will fallback to basic agent without vector search"
                                );

                                // 检查常见问题
                                if e.to_string().contains("embedding") {
                                    tracing::error!(
                                        "💡 Hint: Check your embedding service configuration"
                                    );
                                    tracing::error!(
                                        "   - EMBEDDING_API_KEY: {}",
                                        if self.embedding_api_key.is_empty() {
                                            "❌ Empty"
                                        } else {
                                            "✅ Set"
                                        }
                                    );
                                    tracing::error!(
                                        "   - EMBEDDING_BASE_URL: {}",
                                        self.embedding_url
                                    );
                                    tracing::error!(
                                        "   - EMBEDDING_MODEL: {}",
                                        self.embedding_model
                                    );
                                }

                                if e.to_string().contains("connect")
                                    || e.to_string().contains("database")
                                {
                                    tracing::error!(
                                        "💡 Hint: Check your LanceDB path and permissions"
                                    );
                                    tracing::error!("   - LANCEDB_PATH: {}", self.lancedb_path);
                                }
                            }
                        }
                    } else {
                        // 即使没有文档，也标记为已初始化，保持 LanceDB 可用
                        vector_store_initialized = true;
                        has_documents = false;
                        info!("✅ LanceDB initialized (empty), ready to accept documents");
                    }
                }
                Err(e) => {
                    tracing::warn!("⚠️ Failed to check existing LanceDB data: {}", e);

                    // 如果检查失败但有新文档，仍然尝试初始化
                    if !documents.is_empty() {
                        info!("🔮 Attempting to initialize with new documents anyway...");

                        match store_arc
                            .initialize_with_embeddings(documents, embedding_model.clone())
                            .await
                        {
                            Ok(()) => {
                                vector_store_initialized = true;
                                has_documents = true;
                                info!(
                                    "✅ Successfully initialized LanceDB with {} documents",
                                    doc_count
                                );
                            }
                            Err(e) => {
                                tracing::error!("❌ Failed to initialize LanceDB: {}", e);
                                tracing::warn!(
                                    "🔄 Will fallback to basic agent without vector search"
                                );
                            }
                        }
                    } else {
                        // 检查失败且没有文档，也标记为已初始化
                        vector_store_initialized = true;
                        has_documents = false;
                        info!("✅ LanceDB initialized (empty), ready to accept documents");
                    }
                }
            }
        } else {
            info!("ℹ️ No document store configured, using basic agent");
        }

        // 创建上下文和代理
        let context = RigAgentContext {
            client: client.clone(),
            embedding_model: embedding_model.clone(),
            preamble: final_preamble.clone(),
            temperature: self.temperature,
            openai_model: self.openai_model.clone(),
            needs_rebuild: false, // 初始化时不需要重建
        };

        let rag_agent = if let Some(ref store) = document_store
            && vector_store_initialized
            && has_documents
        {
            if let Some(vector_index) = store.get_vector_index() {
                let total_docs = vector_index.len();
                // RAG 检索数量：取前 3 个最相关文档
                let top_k = total_docs.clamp(1, 3);
                info!(
                    "✅ Using RAG agent with {} total documents, top_k={}",
                    total_docs, top_k
                );

                // 克隆store以避免生命周期问题
                let store_clone = crate::db::DocumentStoreWrapper(Arc::new((**store).clone()));

                client
                    .completion_model(&self.openai_model)
                    .completions_api()
                    .into_agent_builder()
                    .temperature(self.temperature)
                    .preamble(&final_preamble)
                    .dynamic_context(top_k, store_clone)
                    .build()
            } else {
                info!("⚠️ Vector index not available, using basic agent");
                context.build()
            }
        } else {
            if document_store.is_some() && vector_store_initialized && !has_documents {
                info!(
                    "✅ Using basic agent (LanceDB empty, will auto-upgrade when documents added)"
                );
            } else if document_store.is_some() && !vector_store_initialized {
                info!("⚠️ LanceDB initialization failed, using basic agent");
            } else if document_store.is_none() {
                info!("ℹ️ No document store configured, using basic agent");
            }
            context.build()
        };

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

        let client = Client::builder(&self.openai_api_key)
            .base_url(&self.openai_base_url)
            .build()?;

        let embedding_client = Client::builder(&self.embedding_api_key)
            .base_url(&self.embedding_url)
            .build()?;

        let model = embedding_client.embedding_model(&self.embedding_model);

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
                    tracing::info!("✅ Loaded preamble from file: {}", preamble_path);
                    Ok(content)
                }
                Err(e) => {
                    tracing::warn!(
                        "⚠️ Failed to read preamble file {}: {}, using default",
                        preamble_path,
                        e
                    );
                    Ok(self.preamble.clone())
                }
            }
        } else {
            // 没有设置preamble文件，使用初始化时的preamble
            Ok(self.preamble.clone())
        }
    }

    /// 加载文档 - 只从文件系统加载，不同步到数据库
    async fn load_documents(&self) -> anyhow::Result<Vec<FileChunk>> {
        self.load_from_filesystem().await
    }

    /// 从文件系统加载文档
    async fn load_from_filesystem(&self) -> anyhow::Result<Vec<FileChunk>> {
        let documents_dir = std::env::current_dir()?.join(&self.documents_dir);
        info!("📁 Loading documents from {}", documents_dir.display());
        info!("🔍 Excluding preamble file: {}", self.preamble_file);

        // 检查目录是否存在
        if !documents_dir.exists() {
            tracing::warn!(
                "⚠️ Documents directory does not exist: {}",
                documents_dir.display()
            );
            tracing::info!("💡 This is OK if you only want to use existing LanceDB data");
            return Ok(Vec::new());
        }

        let result = FileChunk::load_files(documents_dir.join("*.*"), &self.preamble_file)
            .context("Failed to load documents from filesystem");

        match &result {
            Ok(chunks) => {
                if chunks.is_empty() {
                    info!("📋 No document files found in directory");
                    info!("💡 This is OK if you only want to use existing LanceDB data");
                } else {
                    info!("✅ Successfully loaded {} document files", chunks.len());
                }
            }
            Err(e) => {
                tracing::error!("❌ Failed to load documents: {}", e);
                tracing::warn!("🔄 Will still attempt to use existing LanceDB data");
            }
        }

        result.or_else(|_| Ok(Vec::new())) // 即使加载失败也返回空向量，让程序尝试使用已存在的数据
    }
}
