use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration as StdDuration,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::post,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode,
};
use serde::{Deserialize, Serialize};
use tower_governor::{
    GovernorLayer,
    governor::GovernorConfigBuilder,
    key_extractor::SmartIpKeyExtractor,
};
use tracing::{debug, warn};

use crate::db::{UserRole, UserStore};

/// 禁止使用的占位密钥（与 docs/env.example 一致，公开可查）
const INSECURE_JWT_SECRETS: &[&str] = &[
    "your-secret-key-change-in-production",
    "secret",
    "jwt_secret",
    "changeme",
];

/// JWT Claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // username
    pub user_id: i64,
    pub role: UserRole,
    pub exp: i64, // expiration time
    #[serde(default)]
    pub iat: i64, // issued at
}

/// 登录请求
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// 登录响应
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
    pub role: UserRole,
}

/// JWT工具
pub struct JwtUtil {
    secret: String,
}

impl JwtUtil {
    pub fn new() -> Self {
        let secret = Self::load_secret();
        Self { secret }
    }

    /// 读取并校验 JWT_SECRET：未设置 / 占位值拒绝；长度不强制
    fn load_secret() -> String {
        let secret = std::env::var("JWT_SECRET").unwrap_or_default();
        let trimmed = secret.trim();

        if trimmed.is_empty() {
            panic!(
                "JWT_SECRET is not set. Generate one, e.g. `openssl rand -hex 32`, and put it in .env"
            );
        }
        if INSECURE_JWT_SECRETS
            .iter()
            .any(|s| trimmed.eq_ignore_ascii_case(s))
        {
            panic!(
                "JWT_SECRET is a known insecure placeholder. Set a unique random secret before starting."
            );
        }

        trimmed.to_string()
    }

    fn validation() -> Validation {
        // 只允许 HS256，避免 alg 被篡改
        Validation::new(Algorithm::HS256)
    }

    /// 生成JWT token（默认 24h，缩短失窃/角色变更窗口）
    pub fn generate_token(
        &self,
        user_id: i64,
        username: &str,
        role: UserRole,
    ) -> anyhow::Result<String> {
        let now = Utc::now();
        let expiration = now
            .checked_add_signed(Duration::hours(24))
            .expect("Valid timestamp")
            .timestamp();

        let claims = Claims {
            sub: username.to_string(),
            user_id,
            role,
            exp: expiration,
            iat: now.timestamp(),
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )?;

        Ok(token)
    }

    /// 验证JWT token
    pub fn verify_token(&self, token: &str) -> anyhow::Result<Claims> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &Self::validation(),
        )?;

        Ok(token_data.claims)
    }
}

/// 从反代头或直连 peer 解析客户端 IP（与 SmartIpKeyExtractor 一致）
fn extract_client_ip(headers: &HeaderMap, peer: SocketAddr) -> IpAddr {
    headers
        .get("x-forwarded-for")
        .and_then(|hv| hv.to_str().ok())
        .and_then(|s| s.split(',').find_map(|part| part.trim().parse().ok()))
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|hv| hv.to_str().ok())
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or_else(|| peer.ip())
}

/// 登录处理器
async fn login_handler(
    State(user_store): State<Arc<UserStore>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let client_ip = extract_client_ip(&headers, addr);
    debug!("Login attempt for user: {} from {}", req.username, client_ip);

    let user = match user_store
        .verify_password(&req.username, &req.password)
        .await?
    {
        Some(user) => user,
        None => {
            warn!(
                "Failed login for user '{}' from {}",
                req.username, client_ip
            );
            return Err(AppError::Unauthorized(
                "Invalid username or password".to_string(),
            ));
        }
    };

    // status: 0 禁用，禁用账号禁止签发 token
    if user.status != 1 {
        warn!(
            "Disabled user '{}' tried to login from {}",
            user.username, client_ip
        );
        return Err(AppError::Forbidden("Account disabled".to_string()));
    }

    let jwt_util = JwtUtil::new();
    let token = jwt_util.generate_token(user.id, &user.username, user.role.clone())?;

    debug!(
        "Login successful for user: {} from {}",
        user.username, client_ip
    );

    Ok(Json(LoginResponse {
        token,
        username: user.username,
        role: user.role,
    }))
}

/// 验证当前token（这个handler需要通过中间件提取Claims）
async fn verify_handler(
    axum::extract::Extension(claims): axum::extract::Extension<Claims>,
) -> Json<Claims> {
    Json(claims)
}

/// 创建认证路由
pub fn create_auth_router(user_store: Arc<UserStore>) -> Router {
    // 登录限流：每 IP 突发最多 3 次，之后约每 30 秒才补 1 次
    // 5 秒一次仍会持续打 bcrypt；30 秒把稳态压到约 2 次/分钟
    let login_governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(30)
            .burst_size(3)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("login governor config"),
    );

    // 后台清理限流器中过期的 IP 条目，避免内存缓慢增长
    let limiter = login_governor_conf.limiter().clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(StdDuration::from_secs(60));
        loop {
            interval.tick().await;
            limiter.retain_recent();
        }
    });

    let login_router = Router::new()
        .route("/api/auth/login", post(login_handler))
        .layer(GovernorLayer::new(login_governor_conf))
        .with_state(user_store.clone());

    let verify_router = Router::new()
        .route(
            "/api/auth/verify",
            post(verify_handler)
                .route_layer(axum::middleware::from_fn(require_user_auth_middleware)),
        )
        .with_state(user_store);

    login_router.merge(verify_router)
}

/// 需要用户登录的中间件
pub async fn require_user_auth_middleware(
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    // 提取和验证token
    let token = extract_token(&req)?;
    let jwt_util = JwtUtil::new();
    let claims = jwt_util
        .verify_token(&token)
        .map_err(|_| AppError::Unauthorized("Invalid token".to_string()))?;

    // 将Claims插入到request extensions（供handler使用）
    req.extensions_mut().insert(claims);

    Ok(next.run(req).await)
}

/// JWT认证 + Admin角色检查
pub async fn require_admin_auth_middleware(
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    // 1. 提取和验证token
    let token = extract_token(&req)?;
    let jwt_util = JwtUtil::new();
    let claims = jwt_util
        .verify_token(&token)
        .map_err(|_| AppError::Unauthorized("Invalid token".to_string()))?;

    // 2. 检查Admin角色
    if claims.role != UserRole::Admin {
        warn!(
            "Access denied for user: {} (role: {:?})",
            claims.sub, claims.role
        );
        return Err(AppError::Forbidden("Admin role required".to_string()));
    }

    // 3. 将Claims插入到request extensions（供handler使用）
    req.extensions_mut().insert(claims);

    Ok(next.run(req).await)
}

/// 从请求中提取token
fn extract_token(req: &Request) -> Result<String, AppError> {
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Missing authorization header".to_string()))?;

    if let Some(token) = auth_header.strip_prefix("Bearer ") {
        Ok(token.to_string())
    } else {
        Err(AppError::Unauthorized(
            "Invalid authorization header format".to_string(),
        ))
    }
}

/// 应用错误类型
#[derive(Debug)]
pub enum AppError {
    Unauthorized(String),
    Forbidden(String),
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg),
            AppError::Internal(err) => {
                warn!("Internal error: {:?}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        let body = serde_json::json!({
            "error": message
        });

        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap()
    }
}
