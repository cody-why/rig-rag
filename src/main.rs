use std::{net::SocketAddr, sync::Arc};

use rig_rag::{agent::RigAgent, db::UserStore, web};
use tracing::info;
use tracing_subscriber::fmt::time::OffsetTime;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    let timer = OffsetTime::new(
        time::UtcOffset::from_hms(8, 0, 0).unwrap(),
        time::format_description::well_known::Iso8601::DATE_TIME,
    );
    tracing_subscriber::fmt()
        .with_timer(timer)
        .with_file(true)
        .with_line_number(true)
        .with_target(false)
        .init();
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

    let agent_builder = RigAgent::from_env();
    let agent = agent_builder.build().await.unwrap();

    let document_store = agent.document_store.clone();

    let agent = Arc::new(agent);

    let app = web::create_router(agent, document_store, user_store).await;

    let addr = std::env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    info!(
        "Starting server on http://{}",
        listener.local_addr().unwrap()
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}
