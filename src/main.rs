use std::{net::SocketAddr, sync::Arc};

use rig_rag::{
    agent::RigAgent,
    config::AppConfig,
    db::{ConversationStore, DocumentStore, UserStore},
    utils::logger::init_logger,
    web,
};
use tracing::info;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();

    init_logger().unwrap();
    info!("Starting Agent");

    // 初始化文件备份
    let backup_dir = std::env::var("BACKUP_DIR").unwrap_or_else(|_| "data/backups".to_string());
    if let Err(e) = rig_rag::utils::init_file_backup(&backup_dir).await {
        tracing::warn!("⚠️ Failed to initialize file backup: {}", e);
    } else {
        info!("📁 Initialized file backup at: {}", backup_dir);
    }

    // 初始化用户数据库
    let user_db_path = std::env::var("USER_DB_PATH")
        .unwrap_or_else(|_| "sqlite:data/users.db?mode=rwc".to_string());
    info!("Initializing user database at: {}", user_db_path);
    let user_store = Arc::new(
        UserStore::new(&user_db_path)
            .await
            .expect("Failed to initialize user store"),
    );

    // 加载应用配置
    let config = AppConfig::from_env();

    let agent = RigAgent::new_from_config(&config).await.unwrap();

    // 为路由查询初始化 DocumentStore（供管理/查询接口使用）
    let document_store = Arc::new(DocumentStore::with_config(&config.lancedb));

    let agent = Arc::new(agent);

    let app = web::create_router(agent, document_store, user_store).await;

    let addr = std::env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    info!(
        "Starting server on http://{}",
        listener.local_addr().unwrap()
    );
    close_old_conversations().await;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

async fn close_old_conversations() {
    let conversation_store = ConversationStore::from_env()
        .await
        .expect("Failed to initialize conversation store");

    tokio::spawn(async move {
        loop {
            let closed_count = conversation_store
                .close_old_conversations()
                .await
                .unwrap_or(0);
            if closed_count > 0 {
                // info!("Closed {} conversations (older than 1 day)", closed_count);
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}
