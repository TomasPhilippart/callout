use crate::AppState;
use axum::{
    routing::{delete, get, post},
    Router,
};

pub mod error;

mod agents;
mod ask;
mod notify;
mod ptt;
mod status;

pub use error::AppError;

pub fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/agents/register", post(agents::register))
        .route("/agents/:id", delete(agents::deregister))
        .route("/ask", post(ask::ask))
        .route("/notify", post(notify::notify))
        .route("/status", get(status::status))
        .route("/ptt/start", post(ptt::start))
        .route("/ptt/stop", post(ptt::stop))
        .route("/ptt/toggle", post(ptt::toggle))
        .with_state(state)
}

pub async fn serve(state: AppState) -> anyhow::Result<()> {
    let port = state.config.port;
    let addr = format!("127.0.0.1:{port}");
    tracing::info!(addr = %addr, "callout listening");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, build_app(state)).await?;
    Ok(())
}
