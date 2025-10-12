use std::sync::Arc;

use axum::{Router, extract::Path, http::{HeaderValue, Method}, middleware, response::Html, routing::get};
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};
use tower_http::cors::CorsLayer;

use crate::{agent::RigAgent, db::{DocumentStore, UserStore}, web::{create_auth_router, create_chat_router, create_chat_store, create_user_router, require_admin_auth_middleware, require_user_auth_middleware}};

pub async fn create_router(
    agent: Arc<RigAgent>, document_store: Option<Arc<DocumentStore>>, user_store: Arc<UserStore>,
) -> Router {
    let server_url = "*";
    let cors = CorsLayer::new()
        .allow_origin(server_url.parse::<HeaderValue>().unwrap())
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers(vec![
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ]);

    // 创建聊天历史存储
    let chat_store = create_chat_store();

    // 配置频率限制
    let chat_governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(3)
            .burst_size(10)
            .finish()
            .unwrap(),
    );

    // 用户管理和认证路由（独立state）
    let auth_user_router =
        create_auth_router(user_store.clone()).merge(create_user_router(user_store));

    // 公开路由（不需要认证）
    let public_router = Router::new()
        .route("/", get(serve_index))
        .route("/login", get(serve_login))
        .route("/admin", get(serve_admin))
        .route("/static/{*file}", get(static_file));

    // 需要登录即可访问的查询路由（所有用户）
    let user_query_router = Router::new()
        .merge(crate::web::create_document_query_router())
        .merge(crate::web::create_preamble_query_router())
        .route_layer(middleware::from_fn(require_user_auth_middleware));

    // 需要Admin权限的修改路由
    let admin_mutation_router = Router::new()
        .merge(crate::web::create_document_mutation_router())
        .merge(crate::web::create_preamble_mutation_router())
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            10 * 1024 * 1024,
        )) // 文档上传限制
        .route_layer(middleware::from_fn(require_admin_auth_middleware));

    // 合并所有路由 - 先创建需要AppState的路由组，然后与独立state的路由合并
    let app_state_router = create_chat_router()
        .layer(tower_http::limit::RequestBodyLimitLayer::new(10 * 1024)) // 聊天消息限制为10KB
        .layer(GovernorLayer::new(chat_governor_conf)) // 基于IP的频率限制
        .merge(user_query_router)
        .merge(admin_mutation_router)
        .with_state((agent, document_store, chat_store));

    // 合并所有路由组
    app_state_router
        .merge(public_router)
        .merge(auth_user_router)
        .layer(cors)
}

async fn serve_index() -> Result<Html<String>, axum::http::StatusCode> {
    let file_content = std::fs::read_to_string("static/index.html")
        .map_err(|_| axum::http::StatusCode::NOT_FOUND)?;
    Ok(Html(file_content))
}

async fn serve_admin() -> Result<Html<String>, axum::http::StatusCode> {
    let file_content = std::fs::read_to_string("static/admin.html")
        .map_err(|_| axum::http::StatusCode::NOT_FOUND)?;
    Ok(Html(file_content))
}

async fn serve_login() -> Result<Html<String>, axum::http::StatusCode> {
    let file_content = std::fs::read_to_string("static/login.html")
        .map_err(|_| axum::http::StatusCode::NOT_FOUND)?;
    Ok(Html(file_content))
}

async fn static_file(
    Path(file): Path<String>,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let file_path = file.trim_start_matches('/');

    let full_path = format!("static/{}", file_path);
    let file_content = std::fs::read(&full_path).map_err(|_| axum::http::StatusCode::NOT_FOUND)?;

    let file_type = file_path.split('.').next_back().unwrap_or("bin");
    let mime_type = match file_type {
        "css" => "text/css",
        "js" => "application/javascript",
        "html" => "text/html",
        "md" => "text/markdown",
        "json" => "application/json",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    };

    axum::response::Response::builder()
        .header("Content-Type", mime_type)
        .body(axum::body::Body::from(file_content))
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)
}
