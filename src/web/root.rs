use std::sync::Arc;

use axum::{Router, extract::Path, http::{HeaderValue, Method}, middleware, response::Html, routing::get};
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};
use tower_http::cors::CorsLayer;

use crate::{agent::RigAgent, db::{ConversationStore, DocumentStore, UserStore}, web::*};

pub async fn create_router(
    agent: Arc<RigAgent>, document_store: Arc<DocumentStore>, user_store: Arc<UserStore>,
) -> Router {
    // 初始化对话存储
    let conversation_store = Arc::new(
        ConversationStore::from_env()
            .await
            .expect("Failed to initialize conversation store"),
    );
    let server_url = "*";
    let cors = CorsLayer::new()
        .allow_origin(server_url.parse::<HeaderValue>().unwrap())
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers(vec![
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ]);

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

    // 分别创建不同状态的路由
    let chat_router = create_chat_router()
        .layer(tower_http::limit::RequestBodyLimitLayer::new(10 * 1024)) // 聊天消息限制为10KB
        .layer(GovernorLayer::new(chat_governor_conf)) // 基于IP的频率限制
        .with_state((
            agent.clone(),
            document_store.clone(),
            conversation_store.clone(),
        ));

    let conversation_router = create_conversation_router().with_state((
        agent.clone(),
        document_store.clone(),
        conversation_store,
    ));

    let user_query_router_with_state =
        user_query_router.with_state((agent.clone(), document_store.clone()));

    let admin_mutation_router_with_state =
        admin_mutation_router.with_state((agent, document_store));

    // 合并所有路由组
    public_router
        .merge(auth_user_router)
        .merge(chat_router)
        .merge(conversation_router)
        .merge(user_query_router_with_state)
        .merge(admin_mutation_router_with_state)
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
