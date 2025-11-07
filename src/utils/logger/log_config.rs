use std::error::Error;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct LogConfig {
    pub level: String,
    pub to_file: bool,
    pub to_stdout: bool,
    // pub to_opentelemetry: bool,
    pub file_path: String,
    pub file_name: String,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub log: LogConfig,
}

impl LogConfig {
    pub fn from_file() -> Result<Self, Box<dyn Error>> {
        let path = "data/config.toml";
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config.log)
    }
}
