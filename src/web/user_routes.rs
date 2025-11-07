use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    middleware,
    routing::get,
};
use serde::Serialize;
use tracing::info;

use super::auth_routes::{AppError, Claims, require_user_auth_middleware};
use crate::{
    db::{CreateUserRequest, UpdateUserRequest, User, UserStore},
    web::require_admin_auth_middleware,
};

/// 用户响应
#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub status: i32,
    pub created_at: String,
    pub updated_at: String,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            username: user.username,
            role: user.role.to_string(),
            status: user.status,
            created_at: user.created_at.to_rfc3339(),
            updated_at: user.updated_at.to_rfc3339(),
        }
    }
}

/// 列出所有用户
async fn list_users_handler(
    State(user_store): State<Arc<UserStore>>,
) -> Result<Json<Vec<UserResponse>>, AppError> {
    let users = user_store.list_users().await?;
    let response: Vec<UserResponse> = users.into_iter().map(UserResponse::from).collect();
    Ok(Json(response))
}

/// 获取当前用户信息
async fn get_current_user_handler(
    Extension(claims): Extension<Claims>,
    State(user_store): State<Arc<UserStore>>,
) -> Result<Json<UserResponse>, AppError> {
    let user = user_store
        .get_user_by_id(claims.user_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("User not found")))?;

    Ok(Json(UserResponse::from(user)))
}

/// 获取指定用户信息
async fn get_user_handler(
    Path(id): Path<i64>,
    State(user_store): State<Arc<UserStore>>,
) -> Result<Json<UserResponse>, AppError> {
    let user = user_store
        .get_user_by_id(id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("User not found")))?;

    Ok(Json(UserResponse::from(user)))
}

/// 创建用户
async fn create_user_handler(
    State(user_store): State<Arc<UserStore>>,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<UserResponse>, AppError> {
    info!("Creating new user: {}", req.username);
    let user = user_store.create_user(req).await?;
    Ok(Json(UserResponse::from(user)))
}

/// 更新用户
async fn update_user_handler(
    Path(id): Path<i64>,
    State(user_store): State<Arc<UserStore>>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<UserResponse>, AppError> {
    info!("Updating user with id: {}", id);
    let user = user_store.update_user(id, req).await?;
    Ok(Json(UserResponse::from(user)))
}

/// 删除用户
async fn delete_user_handler(
    Path(id): Path<i64>,
    State(user_store): State<Arc<UserStore>>,
) -> Result<Json<serde_json::Value>, AppError> {
    info!("Deleting user with id: {}", id);
    user_store.delete_user(id).await?;
    Ok(Json(serde_json::json!({
        "message": "User deleted successfully"
    })))
}

/// 创建用户管理路由
pub fn create_user_router(user_store: Arc<UserStore>) -> Router {
    // 需要认证的路由
    let authenticated_routes = Router::new()
        .route("/api/users/me", get(get_current_user_handler))
        .route_layer(middleware::from_fn(require_user_auth_middleware))
        .with_state(user_store.clone());

    // 需要admin权限的路由
    let admin_routes = Router::new()
        .route(
            "/api/users",
            axum::routing::get(list_users_handler).post(create_user_handler),
        )
        .route(
            "/api/users/{id}",
            axum::routing::get(get_user_handler)
                .put(update_user_handler)
                .delete(delete_user_handler),
        )
        .route_layer(middleware::from_fn(require_admin_auth_middleware))
        .with_state(user_store);

    authenticated_routes.merge(admin_routes)
}
