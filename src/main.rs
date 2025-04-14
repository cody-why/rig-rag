use rig_demo::{agent::RigAgent, web};
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();
    info!("Starting Agent");

    let agent = RigAgent::from_env().build().await.unwrap();
    let agent = Arc::new(agent);

    let app = web::create_router(agent).await;

    let addr = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    info!("Starting server on http://{}", addr);
    axum::serve(listener, app.into_make_service()).await.unwrap();
}
