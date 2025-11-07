pub mod document_parser;
pub mod file_backup;
pub mod logger;

pub use document_parser::*;
pub use file_backup::*;

pub fn get_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

pub fn get_env_or_default(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

pub fn get_env_or_panic(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{} is not set", key))
}
