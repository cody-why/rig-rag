use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use tokio::fs;
use tracing::{error, info, warn};

/// 文件备份管理器
/// 负责保存、删除和恢复文档的原始文件副本
#[derive(Debug, Clone)]
pub struct FileBackup {
    backup_dir: PathBuf,
    /// 单个文件最大大小（字节），默认 10MB
    max_file_size: u64,
}

impl FileBackup {
    /// 默认最大文件大小：10MB
    const DEFAULT_MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

    pub fn new<P: AsRef<Path>>(backup_dir: P) -> Self {
        Self {
            backup_dir: backup_dir.as_ref().to_path_buf(),
            max_file_size: Self::DEFAULT_MAX_FILE_SIZE,
        }
    }

    /// 创建带自定义限制的备份管理器
    pub fn with_limits<P: AsRef<Path>>(backup_dir: P, max_file_size: u64) -> Self {
        Self {
            backup_dir: backup_dir.as_ref().to_path_buf(),
            max_file_size,
        }
    }

    /// 初始化备份目录
    pub async fn init(&self) -> Result<()> {
        if !self.backup_dir.exists() {
            fs::create_dir_all(&self.backup_dir)
                .await
                .context("Failed to create backup directory")?;
            info!("📁 Created backup directory: {:?}", self.backup_dir);
        } else {
            info!("📁 Backup directory exists: {:?}", self.backup_dir);
        }
        Ok(())
    }

    /// 保存文档备份
    ///
    /// # Arguments
    /// * `doc_id` - 文档 ID
    /// * `filename` - 原始文件名
    /// * `content` - 文件内容
    ///
    /// # Returns
    /// 返回保存的文件路径
    pub async fn save_backup(
        &self, doc_id: &str, filename: &str, content: &str,
    ) -> Result<PathBuf> {
        // 安全检查 1: 验证 doc_id（只允许字母、数字、下划线、连字符）
        if !Self::is_safe_identifier(doc_id) {
            return Err(anyhow::anyhow!(
                "Invalid doc_id: contains unsafe characters"
            ));
        }

        // 安全检查 2: 检查文件大小
        let content_size = content.len() as u64;
        if content_size > self.max_file_size {
            return Err(anyhow::anyhow!(
                "File size {} exceeds maximum allowed size {}",
                content_size,
                self.max_file_size
            ));
        }

        // 确保备份目录存在
        self.init().await?;

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let safe_filename = self.sanitize_filename(filename);
        let backup_filename = format!("{}_{}_{}", doc_id, timestamp, safe_filename);

        // 安全检查 4: 确保路径在备份目录内
        let backup_path = self.backup_dir.join(&backup_filename);
        if !backup_path.starts_with(&self.backup_dir) {
            return Err(anyhow::anyhow!("Path traversal attempt detected"));
        }

        // 保存文件
        fs::write(&backup_path, content)
            .await
            .context(format!("Failed to write backup file: {:?}", backup_path))?;

        info!(
            "💾 Saved backup: {} -> {:?} ({} bytes)",
            filename, backup_path, content_size
        );

        Ok(backup_path)
    }

    /// 验证标识符是否安全（只允许字母、数字、下划线、连字符）
    pub(crate) fn is_safe_identifier(s: &str) -> bool {
        if s.is_empty() || s.len() > 255 {
            return false;
        }
        s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    }

    /// 删除文档备份
    ///
    /// # Arguments
    /// * `doc_id` - 文档 ID
    ///
    /// # Returns
    /// 返回删除的文件数量
    pub async fn delete_backup(&self, doc_id: &str) -> Result<usize> {
        // 安全检查: 验证 doc_id
        if !Self::is_safe_identifier(doc_id) {
            return Err(anyhow::anyhow!(
                "Invalid doc_id: contains unsafe characters"
            ));
        }

        let mut deleted_count = 0;

        // 检查备份目录是否存在
        if !self.backup_dir.exists() {
            warn!("Backup directory does not exist: {:?}", self.backup_dir);
            return Ok(0);
        }

        // 遍历备份目录，找到所有匹配的文件
        let mut entries = fs::read_dir(&self.backup_dir)
            .await
            .context("Failed to read backup directory")?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            // 安全检查: 确保路径在备份目录内
            if !path.starts_with(&self.backup_dir) {
                warn!("Skipping path outside backup directory: {:?}", path);
                continue;
            }

            // 安全检查: 只处理普通文件，跳过符号链接
            if let Ok(metadata) = entry.metadata().await
                && !metadata.is_file()
            {
                continue;
            }

            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                // 检查文件名格式：必须以 "doc_id_" 开头，避免误删
                let expected_prefix = format!("{}_", doc_id);
                if filename.starts_with(&expected_prefix) {
                    match fs::remove_file(&path).await {
                        Ok(_) => {
                            info!("🗑️  Deleted backup: {:?}", path);
                            deleted_count += 1;
                        },
                        Err(e) => {
                            error!("Failed to delete backup {:?}: {}", path, e);
                        },
                    }
                }
            }
        }

        if deleted_count == 0 {
            warn!("No backup files found for doc_id: {}", doc_id);
        }

        Ok(deleted_count)
    }

    /// 获取文档的备份文件路径
    ///
    /// # Arguments
    /// * `doc_id` - 文档 ID
    ///
    /// # Returns
    /// 返回所有匹配的备份文件路径
    pub async fn get_backup_paths(&self, doc_id: &str) -> Result<Vec<PathBuf>> {
        let mut backup_paths = Vec::new();

        if !self.backup_dir.exists() {
            return Ok(backup_paths);
        }

        let mut entries = fs::read_dir(&self.backup_dir)
            .await
            .context("Failed to read backup directory")?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|n| n.to_str())
                && filename.starts_with(doc_id)
            {
                backup_paths.push(path);
            }
        }

        Ok(backup_paths)
    }

    /// 读取备份文件内容
    ///
    /// # Arguments
    /// * `doc_id` - 文档 ID
    ///
    /// # Returns
    /// 返回 (原始文件名, 内容) 元组
    pub async fn read_backup(&self, doc_id: &str) -> Result<Option<(String, String)>> {
        let backup_paths = self.get_backup_paths(doc_id).await?;

        if backup_paths.is_empty() {
            return Ok(None);
        }

        // 取最新的备份（按文件名排序，因为包含时间戳）
        let latest_backup = backup_paths
            .iter()
            .max_by_key(|p| p.file_name())
            .context("Failed to find latest backup")?;

        let content = fs::read_to_string(latest_backup)
            .await
            .context(format!("Failed to read backup: {:?}", latest_backup))?;

        // 从文件名中提取原始文件名
        // 格式: {doc_id}_{timestamp}_{original_filename}
        let filename = latest_backup
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|s| {
                // 跳过 doc_id 和 timestamp 部分
                let parts: Vec<&str> = s.splitn(3, '_').collect();
                parts.get(2).map(|s| s.to_string())
            })
            .unwrap_or_else(|| "unknown.txt".to_string());

        Ok(Some((filename, content)))
    }

    /// 列出所有备份文件
    ///
    /// # Returns
    /// 返回 (doc_id, 文件名, 大小, 修改时间) 列表
    pub async fn list_all_backups(
        &self,
    ) -> Result<Vec<(String, String, u64, chrono::DateTime<Utc>)>> {
        let mut backups = Vec::new();

        if !self.backup_dir.exists() {
            return Ok(backups);
        }

        let mut entries = fs::read_dir(&self.backup_dir)
            .await
            .context("Failed to read backup directory")?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file()
                && let (Some(filename), Ok(metadata)) = (
                    path.file_name().and_then(|n| n.to_str()),
                    entry.metadata().await,
                )
            {
                // 提取 doc_id (文件名第一部分)
                let doc_id = filename.split('_').next().unwrap_or("unknown").to_string();

                let size = metadata.len();
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|t| {
                        t.duration_since(std::time::UNIX_EPOCH)
                            .ok()
                            .and_then(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0))
                    })
                    .unwrap_or_else(Utc::now);

                backups.push((doc_id, filename.to_string(), size, modified));
            }
        }

        // 按修改时间倒序排列
        backups.sort_by(|a, b| b.3.cmp(&a.3));

        Ok(backups)
    }

    /// 清理旧备份
    /// 每个文档只保留最新的 N 个备份
    ///
    /// # Arguments
    /// * `keep_count` - 每个文档保留的备份数量
    pub async fn cleanup_old_backups(&self, keep_count: usize) -> Result<usize> {
        use std::collections::HashMap;

        if !self.backup_dir.exists() {
            return Ok(0);
        }

        // 按 doc_id 分组所有备份
        let mut doc_backups: HashMap<String, Vec<PathBuf>> = HashMap::new();

        let mut entries = fs::read_dir(&self.backup_dir)
            .await
            .context("Failed to read backup directory")?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file()
                && let Some(filename) = path.file_name().and_then(|n| n.to_str())
            {
                let doc_id = filename.split('_').next().unwrap_or("unknown").to_string();
                doc_backups.entry(doc_id).or_default().push(path);
            }
        }

        let mut deleted_count = 0;

        // 对每个 doc_id 的备份进行清理
        for (doc_id, mut paths) in doc_backups {
            if paths.len() <= keep_count {
                continue;
            }

            // 按文件名排序（文件名包含时间戳）
            paths.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

            // 删除超出保留数量的备份
            for path in paths.iter().skip(keep_count) {
                match fs::remove_file(path).await {
                    Ok(_) => {
                        info!("🧹 Cleaned up old backup: {:?}", path);
                        deleted_count += 1;
                    },
                    Err(e) => {
                        error!("Failed to delete old backup {:?}: {}", path, e);
                    },
                }
            }

            if deleted_count > 0 {
                info!(
                    "🧹 Cleaned {} old backups for doc_id: {}",
                    deleted_count, doc_id
                );
            }
        }

        Ok(deleted_count)
    }

    /// 清理文件名，移除不安全的字符
    pub(crate) fn sanitize_filename(&self, filename: &str) -> String {
        let sanitized: String = filename
            .chars()
            .filter(|c| !c.is_control() && *c != '\u{0000}') // 移除控制字符
            .map(|c| match c {
                // 路径分隔符
                '/' | '\\' => '_',
                // Windows 保留字符
                ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                // 点号开头（隐藏文件）
                '.' => '_',
                // 其他不安全字符
                '\0' | '\n' | '\r' => '_',
                _ => c,
            })
            .collect();

        // 限制长度（文件名最长 100 字符）
        let max_len = 100;
        if sanitized.len() > max_len {
            sanitized.chars().take(max_len).collect()
        } else if sanitized.is_empty() {
            "unnamed".to_string()
        } else {
            sanitized
        }
    }

    /// 获取备份目录总大小
    pub async fn get_total_size(&self) -> Result<u64> {
        let mut total_size = 0u64;

        if !self.backup_dir.exists() {
            return Ok(0);
        }

        let mut entries = fs::read_dir(&self.backup_dir)
            .await
            .context("Failed to read backup directory")?;

        while let Some(entry) = entries.next_entry().await? {
            if let Ok(metadata) = entry.metadata().await
                && metadata.is_file()
            {
                total_size += metadata.len();
            }
        }

        Ok(total_size)
    }
}
