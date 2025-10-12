pub mod document_parser;
pub mod file_backup;

use std::sync::OnceLock;

pub use document_parser::{DocumentParser, DocumentType};
pub use file_backup::FileBackup;

/// 全局 FileBackup 实例
static FILE_BACKUP: OnceLock<FileBackup> = OnceLock::new();

/// 初始化全局 FileBackup
pub async fn init_file_backup(backup_dir: &str) -> anyhow::Result<()> {
    let backup = FileBackup::new(backup_dir);
    backup.init().await?;
    FILE_BACKUP
        .set(backup)
        .map_err(|_| anyhow::anyhow!("FileBackup already initialized"))?;
    Ok(())
}

/// 获取全局 FileBackup 实例
pub fn get_file_backup() -> Option<&'static FileBackup> {
    FILE_BACKUP.get()
}

pub fn get_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

pub fn get_env_or_default(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

pub fn get_env_or_panic(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{} is not set", key))
}
