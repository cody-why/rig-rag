use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use tracing::{debug, info};

/// 对话会话状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ConversationStatus {
    Active,
    Closed,
    Escalated,
}

impl std::fmt::Display for ConversationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConversationStatus::Active => write!(f, "active"),
            ConversationStatus::Closed => write!(f, "closed"),
            ConversationStatus::Escalated => write!(f, "escalated"),
        }
    }
}

/// 消息角色
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageRole::User => write!(f, "user"),
            MessageRole::Assistant => write!(f, "assistant"),
            MessageRole::System => write!(f, "system"),
        }
    }
}

/// 对话会话模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub user_id: String,
    pub status: ConversationStatus,
    pub title: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 对话消息模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: MessageRole,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// 用户交互统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInteractionStats {
    pub user_id: String,
    pub total_conversations: i64,
    pub total_messages: i64,
    pub last_interaction: Option<DateTime<Utc>>,
    pub avg_session_duration: Option<f64>, // 秒
    pub satisfaction_score: Option<f64>,   // 0-5
}

// 手动实现FromRow以支持DateTime转换
impl sqlx::FromRow<'_, SqliteRow> for Conversation {
    fn from_row(row: &SqliteRow) -> sqlx::Result<Self> {
        let created_at_ts: i64 = row.try_get("created_at")?;
        let updated_at_ts: i64 = row.try_get("updated_at")?;

        let created_at = DateTime::from_timestamp(created_at_ts, 0).ok_or_else(|| {
            sqlx::Error::Decode(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid timestamp created_at",
            )))
        })?;

        let updated_at = DateTime::from_timestamp(updated_at_ts, 0).ok_or_else(|| {
            sqlx::Error::Decode(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid timestamp updated_at",
            )))
        })?;

        Ok(Conversation {
            id: row.try_get("id")?,
            user_id: row.try_get("user_id")?,
            status: row.try_get("status")?,
            title: row.try_get("title")?,
            metadata: row
                .try_get("metadata")
                .ok()
                .and_then(|s: String| serde_json::from_str(&s).ok()),
            created_at,
            updated_at,
        })
    }
}

impl sqlx::FromRow<'_, SqliteRow> for ConversationMessage {
    fn from_row(row: &SqliteRow) -> sqlx::Result<Self> {
        let created_at_ts: i64 = row.try_get("created_at")?;

        let created_at = DateTime::from_timestamp(created_at_ts, 0).ok_or_else(|| {
            sqlx::Error::Decode(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid timestamp created_at",
            )))
        })?;

        Ok(ConversationMessage {
            id: row.try_get("id")?,
            conversation_id: row.try_get("conversation_id")?,
            role: row.try_get("role")?,
            content: row.try_get("content")?,
            metadata: row
                .try_get("metadata")
                .ok()
                .and_then(|s: String| serde_json::from_str(&s).ok()),
            created_at,
        })
    }
}

/// 创建对话请求
#[derive(Debug, Clone, Deserialize)]
pub struct CreateConversationRequest {
    pub user_id: String,
    pub title: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// 创建消息请求
#[derive(Debug, Clone, Deserialize)]
pub struct CreateMessageRequest {
    pub conversation_id: String,
    pub role: MessageRole,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
}

/// 更新对话请求
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateConversationRequest {
    pub status: Option<ConversationStatus>,
    pub title: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// 对话存储
pub struct ConversationStore {
    pool: SqlitePool,
}

impl ConversationStore {
    pub async fn from_env() -> Result<Self> {
        let conversation_db_path = std::env::var("CONVERSATION_DB_PATH")
            .unwrap_or_else(|_| "sqlite:data/conversations.db?mode=rwc".to_string());
        Self::new(&conversation_db_path).await
    }

    /// 创建新的对话存储实例
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = SqlitePool::connect(database_url)
            .await
            .context("Failed to connect to conversation database")?;

        let store = Self { pool };
        store.init_database().await?;
        Ok(store)
    }

    /// 初始化数据库表
    async fn init_database(&self) -> Result<()> {
        // 检查表是否已存在
        let conversations_exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='conversations'",
        )
        .fetch_one(&self.pool)
        .await?;

        let is_new_table = conversations_exists == 0;

        if is_new_table {
            // 创建对话会话表
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS conversations (
                    id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    status TEXT NOT NULL CHECK(status IN ('active', 'closed', 'escalated')),
                    title TEXT,
                    metadata TEXT, -- JSON string
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_conversations_user_id ON conversations(user_id);
                CREATE INDEX IF NOT EXISTS idx_conversations_status ON conversations(status);
                CREATE INDEX IF NOT EXISTS idx_conversations_created_at ON conversations(created_at);
                CREATE INDEX IF NOT EXISTS idx_conversations_status_updated_at ON conversations(status, updated_at);
                CREATE INDEX IF NOT EXISTS idx_conversations_status_created_at ON conversations(status, created_at);
                "#,
            )
            .execute(&self.pool)
            .await
            .context("Failed to create conversations table")?;

            // 创建对话消息表
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS conversation_messages (
                    id TEXT PRIMARY KEY,
                    conversation_id TEXT NOT NULL,
                    role TEXT NOT NULL CHECK(role IN ('user', 'assistant', 'system')),
                    content TEXT NOT NULL,
                    metadata TEXT, -- JSON string
                    created_at INTEGER NOT NULL,
                    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_messages_conversation_id ON conversation_messages(conversation_id);
                CREATE INDEX IF NOT EXISTS idx_messages_created_at ON conversation_messages(created_at);
                "#,
            )
            .execute(&self.pool)
            .await
            .context("Failed to create conversation_messages table")?;

            info!("Conversation database tables created successfully");
        }

        Ok(())
    }

    /// 创建新对话
    pub async fn create_conversation(
        &self, req: CreateConversationRequest,
    ) -> Result<Conversation> {
        let id = nanoid::nanoid!();
        let now = Utc::now();
        let timestamp = now.timestamp();

        let metadata_json = req
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("Failed to serialize metadata")?;

        sqlx::query(
            r#"
            INSERT INTO conversations (id, user_id, status, title, metadata, created_at, updated_at)
            VALUES (?, ?, 'active', ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&req.user_id)
        .bind(&req.title)
        .bind(&metadata_json)
        .bind(timestamp)
        .bind(timestamp)
        .execute(&self.pool)
        .await
        .context("Failed to insert conversation")?;

        debug!("Created conversation: {} for user: {}", id, req.user_id);

        Ok(Conversation {
            id,
            user_id: req.user_id,
            status: ConversationStatus::Active,
            title: req.title,
            metadata: req.metadata,
            created_at: now,
            updated_at: now,
        })
    }

    /// 获取或创建活跃对话
    pub async fn get_or_create_active_conversation(&self, user_id: &str) -> Result<Conversation> {
        // 首先查找活跃的对话
        let conversation = sqlx::query_as::<_, Conversation>(
            r#"
            SELECT id, user_id, status, title, metadata, created_at, updated_at
            FROM conversations
            WHERE user_id = ? AND status = 'active'
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .context("Failed to query active conversation")?;

        if let Some(conversation) = conversation {
            return Ok(conversation);
        }

        // 如果没有活跃对话，创建一个新的
        let req = CreateConversationRequest {
            user_id: user_id.to_string(),
            title: None,
            metadata: None,
        };
        self.create_conversation(req).await
    }

    /// 添加消息到对话
    pub async fn add_message(&self, req: CreateMessageRequest) -> Result<ConversationMessage> {
        let id = nanoid::nanoid!();
        let now = Utc::now();
        let timestamp = now.timestamp();

        let metadata_json = req
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("Failed to serialize metadata")?;

        // 开始事务
        let mut tx = self.pool.begin().await?;

        // 插入消息
        sqlx::query(
            r#"
            INSERT INTO conversation_messages (id, conversation_id, role, content, metadata, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&req.conversation_id)
        .bind(req.role.to_string())
        .bind(&req.content)
        .bind(&metadata_json)
        .bind(timestamp)
        .execute(&mut *tx)
        .await
        .context("Failed to insert message")?;

        // 更新对话的最后更新时间
        sqlx::query(
            r#"
            UPDATE conversations 
            SET updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(timestamp)
        .bind(&req.conversation_id)
        .execute(&mut *tx)
        .await
        .context("Failed to update conversation")?;

        // 提交事务
        tx.commit().await.context("Failed to commit transaction")?;

        debug!(
            "Added message: {} to conversation: {}",
            id, req.conversation_id
        );

        Ok(ConversationMessage {
            id,
            conversation_id: req.conversation_id,
            role: req.role,
            content: req.content,
            metadata: req.metadata,
            created_at: now,
        })
    }

    /// 获取对话消息历史
    pub async fn get_conversation_messages(
        &self, conversation_id: &str, limit: Option<i64>, offset: Option<i64>,
    ) -> Result<Vec<ConversationMessage>> {
        let limit = limit.unwrap_or(50);
        let offset = offset.unwrap_or(0);

        let messages = sqlx::query_as::<_, ConversationMessage>(
            r#"
            SELECT id, conversation_id, role, content, metadata, created_at
            FROM conversation_messages
            WHERE conversation_id = ?
            ORDER BY created_at ASC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(conversation_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .context("Failed to query conversation messages")?;

        Ok(messages)
    }

    /// 获取用户的对话列表
    pub async fn get_user_conversations(
        &self, user_id: &str, limit: Option<i64>, offset: Option<i64>,
    ) -> Result<Vec<Conversation>> {
        let limit = limit.unwrap_or(20);
        let offset = offset.unwrap_or(0);

        let conversations = sqlx::query_as::<_, Conversation>(
            r#"
            SELECT id, user_id, status, title, metadata, created_at, updated_at
            FROM conversations
            WHERE user_id = ?
            ORDER BY updated_at DESC, created_at DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .context("Failed to query user conversations")?;

        Ok(conversations)
    }

    /// 更新对话状态
    pub async fn update_conversation(
        &self, conversation_id: &str, req: UpdateConversationRequest,
    ) -> Result<Conversation> {
        let mut set_clauses = Vec::new();

        let now = Utc::now();
        let timestamp = now.timestamp();

        // 处理状态更新
        let status_str = req.status.as_ref().map(|s| s.to_string());
        if status_str.is_some() {
            set_clauses.push("status = ?");
        }

        // 处理标题更新
        if req.title.is_some() {
            set_clauses.push("title = ?");
        }

        // 处理元数据更新
        let metadata_json = req
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("Failed to serialize metadata")?;
        if metadata_json.is_some() {
            set_clauses.push("metadata = ?");
        }

        // 如果没有任何字段需要更新，直接返回当前对话
        if set_clauses.is_empty() {
            return self
                .get_conversation_by_id(conversation_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Conversation not found"));
        }

        // 总是更新 updated_at
        set_clauses.push("updated_at = ?");

        // 构建 SQL
        let sql = format!(
            "UPDATE conversations SET {} WHERE id = ?",
            set_clauses.join(", ")
        );

        // 绑定参数
        let mut query = sqlx::query(&sql);

        if let Some(ref status) = status_str {
            query = query.bind(status);
        }
        if let Some(ref title) = req.title {
            query = query.bind(title);
        }
        if let Some(ref metadata) = metadata_json {
            query = query.bind(metadata);
        }
        query = query.bind(timestamp).bind(conversation_id);

        // 执行更新
        let result = query
            .execute(&self.pool)
            .await
            .context("Failed to update conversation")?;

        if result.rows_affected() == 0 {
            return Err(anyhow::anyhow!("Conversation not found"));
        }

        // 查询并返回更新后的对话
        let conversation = self
            .get_conversation_by_id(conversation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Conversation not found"))?;
        debug!("Updated conversation: {}", conversation_id);
        Ok(conversation)
    }

    /// 根据ID获取对话
    pub async fn get_conversation_by_id(
        &self, conversation_id: &str,
    ) -> Result<Option<Conversation>> {
        let conversation = sqlx::query_as::<_, Conversation>(
            r#"
            SELECT id, user_id, status, title, metadata, created_at, updated_at
            FROM conversations
            WHERE id = ?
            "#,
        )
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await
        .context("Failed to query conversation by id")?;

        Ok(conversation)
    }

    /// 获取用户交互统计
    pub async fn get_user_interaction_stats(&self, user_id: &str) -> Result<UserInteractionStats> {
        let stats = sqlx::query_as::<_, (i64, i64, Option<i64>)>(
            r#"
            SELECT 
                COUNT(DISTINCT c.id) as total_conversations,
                COUNT(m.id) as total_messages,
                MAX(c.updated_at) as last_interaction
            FROM conversations c
            LEFT JOIN conversation_messages m ON c.id = m.conversation_id
            WHERE c.user_id = ?
            "#,
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .context("Failed to query user interaction stats")?;

        let last_interaction = stats.2.and_then(|ts| DateTime::from_timestamp(ts, 0));

        Ok(UserInteractionStats {
            user_id: user_id.to_string(),
            total_conversations: stats.0,
            total_messages: stats.1,
            last_interaction,
            avg_session_duration: None,
            satisfaction_score: None,
        })
    }

    /// 删除对话（硬删除, 会话及其消息）
    pub async fn delete_conversation(&self, conversation_id: &str) -> Result<()> {
        // 使用事务确保会话与其消息要么一起删除，要么都不删除
        let mut tx = self.pool.begin().await?;

        // 先删除消息（即便启用了外键级联，这里也主动清理以兼容未启用外键约束的环境）
        let deleted_messages = sqlx::query(
            r#"
            DELETE FROM conversation_messages
            WHERE conversation_id = ?
            "#,
        )
        .bind(conversation_id)
        .execute(&mut *tx)
        .await
        .context("Failed to delete conversation messages")?
        .rows_affected();

        // 删除会话本身
        let deleted_conversations = sqlx::query(
            r#"
            DELETE FROM conversations
            WHERE id = ?
            "#,
        )
        .bind(conversation_id)
        .execute(&mut *tx)
        .await
        .context("Failed to delete conversation")?
        .rows_affected();

        if deleted_conversations == 0 {
            // 回滚事务并返回会话不存在
            tx.rollback().await.ok();
            return Err(anyhow::anyhow!("Conversation not found"));
        }

        // 提交事务
        tx.commit()
            .await
            .context("Failed to commit delete transaction")?;

        info!(
            "Deleted conversation: {} ({} messages removed)",
            conversation_id, deleted_messages
        );
        Ok(())
    }

    /// 清理旧数据（可选功能）
    pub async fn cleanup_old_data(&self, days_to_keep: i64) -> Result<u64> {
        let cutoff_timestamp = Utc::now().timestamp() - (days_to_keep * 24 * 60 * 60);

        let deleted_count = sqlx::query(
            r#"
            DELETE FROM conversations 
            WHERE status = 'closed' AND updated_at < ?
            "#,
        )
        .bind(cutoff_timestamp)
        .execute(&self.pool)
        .await
        .context("Failed to cleanup old conversations")?
        .rows_affected();

        info!("Cleaned up {} old conversations", deleted_count);
        Ok(deleted_count)
    }
    /// 关闭超过一天的对话
    pub async fn close_old_conversations(&self) -> Result<u64> {
        // 计算一天前的时间戳（以秒为单位）
        let one_day_ago = chrono::Utc::now().timestamp() - 24 * 60 * 60;

        let closed_count = sqlx::query(
            r#"
            UPDATE conversations
            SET status = 'closed'
            WHERE updated_at < ?
            AND status = 'active'
            "#,
        )
        .bind(one_day_ago)
        .execute(&self.pool)
        .await
        .context("Failed to close conversation")?
        .rows_affected() as u64;

        Ok(closed_count)
    }

    /// 智能检测消息内容是否表示对话结束（简化版本）
    pub fn detect_conversation_end_indicators(message: &str) -> bool {
        let msg = message.to_lowercase();

        // 简化的结束语检测
        let end_words = [
            "再见",
            "拜拜",
            "结束",
            "完成",
            "好了",
            "谢谢",
            "感谢",
            "没问题",
            "明白了",
            "搞定",
            "解决",
            "bye",
            "goodbye",
            "thanks",
            "thank you",
            "done",
            "finished",
            "completed",
            "perfect",
            "great",
        ];

        end_words.iter().any(|word| msg.contains(word))
    }

    /// 智能关闭对话（简化版本）
    pub async fn smart_close_conversation_if_needed(
        &self, conversation_id: &str, user_message: &str,
    ) -> Result<bool> {
        if !Self::detect_conversation_end_indicators(user_message) {
            return Ok(false);
        }

        // 直接关闭对话
        let _ = self
            .update_conversation(
                conversation_id,
                crate::db::UpdateConversationRequest {
                    status: Some(ConversationStatus::Closed),
                    title: None,
                    metadata: Some(serde_json::json!({
                        "auto_closed_reason": "user_indicated_end",
                        "closed_at": Utc::now().to_rfc3339()
                    })),
                },
            )
            .await;

        info!(
            "Smart-closed conversation {} due to end indicators",
            conversation_id
        );
        Ok(true)
    }

    /// 获取对话统计信息
    pub async fn get_conversation_stats(&self) -> Result<ConversationStats> {
        let stats = sqlx::query_as::<_, (i64, i64, i64, i64)>(
            r#"
            SELECT 
                COUNT(*) as total_conversations,
                COUNT(CASE WHEN status = 'active' THEN 1 END) as active_conversations,
                COUNT(CASE WHEN status = 'closed' THEN 1 END) as closed_conversations,
                COUNT(CASE WHEN status = 'escalated' THEN 1 END) as escalated_conversations
            FROM conversations
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .context("Failed to query conversation stats")?;

        let message_stats = sqlx::query_as::<_, (i64,)>(
            r#"
            SELECT COUNT(*) as total_messages
            FROM conversation_messages
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .context("Failed to query message stats")?;

        let today_stats = sqlx::query_as::<_, (i64,)>(
            r#"
            SELECT COUNT(*) as today_conversations
            FROM conversations
            WHERE created_at >= ?
            "#,
        )
        .bind(Utc::now().timestamp() - 24 * 60 * 60) // 24小时前
        .fetch_one(&self.pool)
        .await
        .context("Failed to query today stats")?;

        Ok(ConversationStats {
            total_conversations: stats.0,
            active_conversations: stats.1,
            closed_conversations: stats.2,
            escalated_conversations: stats.3,
            total_messages: message_stats.0,
            today_conversations: today_stats.0,
        })
    }

    /// 获取所有对话（管理员功能）
    pub async fn get_all_conversations(
        &self, limit: Option<i64>, offset: Option<i64>, search: Option<&str>,
    ) -> Result<Vec<Conversation>> {
        let limit = limit.unwrap_or(20);
        let offset = offset.unwrap_or(0);

        let conversations = if let Some(search_term) = search {
            sqlx::query_as::<_, Conversation>(
                r#"
                SELECT id, user_id, status, title, metadata, created_at, updated_at
                FROM conversations
                WHERE user_id LIKE ? OR id LIKE ?
                ORDER BY updated_at DESC, created_at DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(format!("%{}%", search_term))
            .bind(format!("%{}%", search_term))
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .context("Failed to query conversations with search")?
        } else {
            sqlx::query_as::<_, Conversation>(
                r#"
                SELECT id, user_id, status, title, metadata, created_at, updated_at
                FROM conversations
                ORDER BY updated_at DESC, created_at DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .context("Failed to query conversations")?
        };

        Ok(conversations)
    }
}

/// 对话统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationStats {
    pub total_conversations: i64,
    pub active_conversations: i64,
    pub closed_conversations: i64,
    pub escalated_conversations: i64,
    pub total_messages: i64,
    pub today_conversations: i64,
}
