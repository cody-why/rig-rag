use rig_rag::{agent::RigAgent, web};
use std::sync::Arc;
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

    let agent_builder = RigAgent::from_env();
    let agent = agent_builder.build().await.unwrap();

    let document_store = agent.document_store.clone();

    let agent = Arc::new(agent);

    let app = web::create_router(agent, document_store).await;

    let addr = std::env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    info!(
        "Starting server on http://{}",
        listener.local_addr().unwrap()
    );
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}
