use std::env;

/// LanceDB 配置
#[derive(Debug, Clone)]
pub struct LanceDbConfig {
    pub path: String,
    pub table_name: String,
}

impl LanceDbConfig {
    /// 从环境变量创建配置
    pub fn from_env() -> Self {
        Self {
            path: env::var("LANCEDB_PATH").unwrap_or_else(|_| "data/lancedb".to_string()),
            table_name: "documents".to_string(),
        }
    }

    /// 获取完整的表路径
    pub fn table_path(&self) -> String {
        format!("{}/{}", self.path, self.table_name)
    }
}

/// 应用配置
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub lancedb: LanceDbConfig,
    pub preamble_file: String,
    pub temperature: f64,
    pub documents_dir: String,
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
            lancedb: LanceDbConfig::from_env(),
            preamble_file: env::var("PREAMBLE_FILE")
                .unwrap_or_else(|_| "data/preamble.md".to_string()),
            temperature: env::var("TEMPERATURE")
                .unwrap_or_else(|_| "0.7".to_string())
                .parse()
                .unwrap_or(0.7),
            documents_dir: env::var("DOCUMENTS_DIR")
                .unwrap_or_else(|_| "data/documents".to_string()),
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
