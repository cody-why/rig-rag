use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use tracing::{debug, info};

/// 用户角色
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Admin,
    User,
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserRole::Admin => write!(f, "admin"),
            UserRole::User => write!(f, "user"),
        }
    }
}

/// 用户模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: UserRole,
    pub status: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// 手动实现FromRow以支持DateTime转换
impl sqlx::FromRow<'_, SqliteRow> for User {
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

        Ok(User {
            id: row.try_get("id")?,
            username: row.try_get("username")?,
            password_hash: row.try_get("password_hash")?,
            role: row.try_get("role")?,
            status: row.try_get("status")?,
            created_at,
            updated_at,
        })
    }
}

/// 创建用户请求
#[derive(Debug, Clone, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: Option<UserRole>,
    pub status: Option<i32>,
}

/// 更新用户请求
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateUserRequest {
    pub password: Option<String>,
    pub status: Option<i32>, // 0: disabled, 1: enabled
    pub role: Option<UserRole>,
}

/// 用户存储
pub struct UserStore {
    pool: SqlitePool,
}

impl UserStore {
    /// 创建新的用户存储实例
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = SqlitePool::connect(database_url)
            .await
            .context("Failed to connect to user database")?;

        let store = Self { pool };
        store.init_database().await?;
        Ok(store)
    }

    /// 初始化数据库表
    async fn init_database(&self) -> Result<()> {
        // 检查表是否已存在
        let table_exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='users'",
        )
        .fetch_one(&self.pool)
        .await?;

        let is_new_table = table_exists == 0;

        // 只在第一次创建表时设置序列起始值
        if is_new_table {
            // 创建用户表和索引
            sqlx::query(
                r#"
            CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL CHECK(role IN ('admin', 'user')),
                status INTEGER NOT NULL CHECK(status IN (0, 1)),
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
            "#,
            )
            .execute(&self.pool)
            .await
            .context("Failed to initialize users table")?;

            sqlx::query("INSERT INTO sqlite_sequence (name, seq) VALUES ('users', 1000)")
                .execute(&self.pool)
                .await
                .context("Failed to set sequence start value")?;
        }

        // 检查是否有admin用户，如果没有则创建默认admin
        let admin_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin'")
                .fetch_one(&self.pool)
                .await?;

        if admin_count == 0 {
            info!("No admin user found, creating default admin");
            let default_password =
                std::env::var("DEFAULT_ADMIN_PASSWORD").unwrap_or_else(|_| "aaa111".to_string());

            self.create_user(CreateUserRequest {
                username: "admin".to_string(),
                password: default_password,
                role: Some(UserRole::Admin),
                status: Some(1),
            })
            .await?;

            info!("Default admin user created (username: admin)");
        }

        Ok(())
    }

    /// 创建用户
    pub async fn create_user(&self, req: CreateUserRequest) -> Result<User> {
        let password_hash =
            bcrypt::hash(&req.password, bcrypt::DEFAULT_COST).context("Failed to hash password")?;

        let role = req.role.unwrap_or(UserRole::User);
        let status = req.status.unwrap_or(1);
        let now = Utc::now();
        let timestamp = now.timestamp();

        let id = sqlx::query(
            r#"
            INSERT INTO users (username, password_hash, role, status, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&req.username)
        .bind(&password_hash)
        .bind(role.to_string())
        .bind(status)
        .bind(timestamp)
        .bind(timestamp)
        .execute(&self.pool)
        .await
        .context("Failed to insert user")?
        .last_insert_rowid();

        debug!("Created user: {} with id: {}", req.username, id);

        Ok(User {
            id,
            username: req.username,
            password_hash,
            role,
            status,
            created_at: now,
            updated_at: now,
        })
    }

    /// 根据用户名查找用户
    pub async fn get_user_by_username(&self, username: &str) -> Result<Option<User>> {
        let user = sqlx::query_as::<_, User>(
            r#"
            SELECT id, username, password_hash, role, status, created_at, updated_at
            FROM users
            WHERE username = ?
            "#,
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .context("Failed to query user")?;

        Ok(user)
    }

    /// 根据ID查找用户
    pub async fn get_user_by_id(&self, id: i64) -> Result<Option<User>> {
        let user = sqlx::query_as::<_, User>(
            r#"
            SELECT id, username, password_hash, role, status, created_at, updated_at
            FROM users
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("Failed to query user by id")?;

        Ok(user)
    }

    /// 验证用户密码
    pub async fn verify_password(&self, username: &str, password: &str) -> Result<Option<User>> {
        let user = self.get_user_by_username(username).await?;

        if let Some(user) = user
            && bcrypt::verify(password, &user.password_hash).context("Failed to verify password")?
        {
            return Ok(Some(user));
        }

        Ok(None)
    }

    /// 列出所有用户
    pub async fn list_users(&self) -> Result<Vec<User>> {
        let users = sqlx::query_as::<_, User>(
            r#"
            SELECT id, username, password_hash, role, status, created_at, updated_at
            FROM users
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("Failed to list users")?;

        Ok(users)
    }

    /// 更新用户
    pub async fn update_user(&self, id: i64, req: UpdateUserRequest) -> Result<User> {
        let mut set_clauses = Vec::new();

        let now = Utc::now();
        let timestamp = now.timestamp();

        // 处理密码更新
        let password_hash = if let Some(password) = req.password {
            let hash =
                bcrypt::hash(&password, bcrypt::DEFAULT_COST).context("Failed to hash password")?;
            set_clauses.push("password_hash = ?");
            Some(hash)
        } else {
            None
        };

        // 处理角色更新
        let role_str = req.role.as_ref().map(|r| r.to_string());
        if role_str.is_some() {
            set_clauses.push("role = ?");
        }

        // 处理状态更新
        if req.status.is_some() {
            set_clauses.push("status = ?");
        }

        // 如果没有任何字段需要更新，直接返回当前用户
        if set_clauses.is_empty() {
            return self
                .get_user_by_id(id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("User not found"));
        }

        // 总是更新 updated_at
        set_clauses.push("updated_at = ?");

        // 构建 SQL
        let sql = format!("UPDATE users SET {} WHERE id = ?", set_clauses.join(", "));

        // 绑定参数
        let mut query = sqlx::query(&sql);

        if let Some(ref hash) = password_hash {
            query = query.bind(hash);
        }
        if let Some(ref role) = role_str {
            query = query.bind(role);
        }
        if let Some(status) = req.status {
            query = query.bind(status);
        }
        query = query.bind(timestamp).bind(id);

        // 执行更新
        let result = query
            .execute(&self.pool)
            .await
            .context("Failed to update user")?;

        if result.rows_affected() == 0 {
            return Err(anyhow::anyhow!("User not found"));
        }

        // 查询并返回更新后的用户
        let user = self
            .get_user_by_id(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("User not found"))?;

        debug!("Updated user: {}", user.username);
        Ok(user)
    }

    /// 删除用户
    pub async fn delete_user(&self, id: i64) -> Result<()> {
        // 不允许删除最后一个admin用户
        if id == 1001 {
            return Err(anyhow::anyhow!("Cannot delete the admin user"));
        }
        let user = self
            .get_user_by_id(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("User not found"))?;

        if user.role == UserRole::Admin {
            let admin_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin'")
                    .fetch_one(&self.pool)
                    .await?;

            if admin_count <= 1 {
                return Err(anyhow::anyhow!("Cannot delete the last admin user"));
            }
        }

        sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .context("Failed to delete user")?;

        info!("Deleted user: {} (id: {})", user.username, id);
        Ok(())
    }
}
