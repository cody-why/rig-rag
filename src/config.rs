use std::env;

use qdrant_client::qdrant::Distance;

/// Qdrant 配置
#[derive(Debug, Clone)]
pub struct QdrantConfig {
    pub url: String,
    pub api_key: Option<String>,
    pub collection_name: String,
    pub vector_size: usize,
    pub distance: Distance,
}

impl QdrantConfig {
    /// 从环境变量创建配置
    pub fn from_env() -> Self {
        let url = env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string());
        let collection_name =
            env::var("QDRANT_COLLECTION").unwrap_or_else(|_| "rig_documents".to_string());
        let api_key = env::var("QDRANT_API_KEY").ok().filter(|v| !v.is_empty());
        let vector_size = env::var("QDRANT_VECTOR_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1024);
        let distance = env::var("QDRANT_DISTANCE")
            .ok()
            .and_then(|value| Self::parse_distance(&value))
            .unwrap_or(Distance::Cosine);

        Self {
            url,
            api_key,
            collection_name,
            vector_size,
            distance,
        }
    }

    fn parse_distance(value: &str) -> Option<Distance> {
        match value.trim().to_lowercase().as_str() {
            "cosine" => Some(Distance::Cosine),
            "dot" | "dotproduct" | "dot_product" => Some(Distance::Dot),
            "euclid" | "euclidean" => Some(Distance::Euclid),
            "manhattan" | "l1" => Some(Distance::Manhattan),
            _ => None,
        }
    }
}

/// 应用配置
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub qdrant: QdrantConfig,
    pub preamble_file: String,
    pub temperature: f64,
    pub documents_dir: String,
    /// 相对 DOCUMENTS_DIR 排除的目录（名称或相对路径，逗号分隔）
    pub documents_exclude_dirs: Vec<String>,
    /// 轮询 DOCUMENTS_DIR 变更的间隔（秒），最小 5
    pub documents_sync_interval_secs: u64,
    /// RAG 每次检索注入的文档切片数（dynamic_context top_k）
    pub rag_top_k: usize,
    pub openai_api_key: String,
    pub openai_base_url: String,
    pub openai_model: String,
    pub embedding_api_key: String,
    pub embedding_url: String,
    pub embedding_model: String,
}

impl AppConfig {
    /// 从环境变量创建配置
    pub fn from_env() -> Self {
        let openai_api_key = env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");
        let openai_base_url = env::var("OPENAI_BASE_URL").expect("OPENAI_BASE_URL must be set");

        Self {
            qdrant: QdrantConfig::from_env(),
            preamble_file: env::var("PREAMBLE_FILE")
                .unwrap_or_else(|_| "data/preamble.md".to_string()),
            temperature: env::var("TEMPERATURE")
                .unwrap_or_else(|_| "0.7".to_string())
                .parse()
                .unwrap_or(0.7),
            documents_dir: env::var("DOCUMENTS_DIR")
                .unwrap_or_else(|_| "data/documents".to_string()),
            documents_exclude_dirs: parse_csv_list(
                &env::var("DOCUMENTS_EXCLUDE_DIRS").unwrap_or_default(),
            ),
            documents_sync_interval_secs: env::var("DOCUMENTS_SYNC_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30)
                .max(5),
            rag_top_k: env::var("RAG_TOP_K")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5)
                .max(1),
            openai_api_key: openai_api_key.clone(),
            openai_base_url: openai_base_url.clone(),
            openai_model: env::var("OPENAI_MODEL").expect("OPENAI_MODEL must be set"),
            embedding_api_key: env::var("EMBEDDING_API_KEY")
                .unwrap_or_else(|_| openai_api_key.clone()),
            embedding_url: env::var("EMBEDDING_BASE_URL")
                .unwrap_or_else(|_| openai_base_url.clone()),
            embedding_model: env::var("EMBEDDING_MODEL")
                .unwrap_or_else(|_| "text-embedding-ada-002".to_string()),
        }
    }
}

fn parse_csv_list(raw: &str) -> Vec<String> {
    raw.split([',', ';'])
        .map(|s| s.trim().trim_matches('/').replace('\\', "/"))
        .filter(|s| !s.is_empty())
        .collect()
}
