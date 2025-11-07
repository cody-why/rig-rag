use std::sync::Arc;

use axum::{
    Json, Router,
    body::Body,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::post,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::db::{UserRole, UserStore};

/// JWT Claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // username
    pub user_id: i64,
    pub role: UserRole,
    pub exp: i64, // expiration time
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
        let secret = std::env::var("JWT_SECRET")
            .unwrap_or_else(|_| "your-secret-key-change-in-production".to_string());
        Self { secret }
    }

    /// 生成JWT token
    pub fn generate_token(
        &self,
        user_id: i64,
        username: &str,
        role: UserRole,
    ) -> anyhow::Result<String> {
        let expiration = Utc::now()
            .checked_add_signed(Duration::days(7))
            .expect("Valid timestamp")
            .timestamp();

        let claims = Claims {
            sub: username.to_string(),
            user_id,
            role,
            exp: expiration,
        };

        let token = encode(
            &Header::default(),
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
            &Validation::default(),
        )?;

        Ok(token_data.claims)
    }
}

/// 登录处理器
async fn login_handler(
    State(user_store): State<Arc<UserStore>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    debug!("Login attempt for user: {}", req.username);

    let user = user_store
        .verify_password(&req.username, &req.password)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid username or password".to_string()))?;

    let jwt_util = JwtUtil::new();
    let token = jwt_util.generate_token(user.id, &user.username, user.role.clone())?;

    debug!("Login successful for user: {}", user.username);

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
    Router::new()
        .route("/api/auth/login", post(login_handler))
        .route(
            "/api/auth/verify",
            post(verify_handler)
                .route_layer(axum::middleware::from_fn(require_user_auth_middleware)),
        )
        .with_state(user_store)
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
