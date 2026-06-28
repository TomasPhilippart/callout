use axum::{routing::{delete, get, post}, Router};
use crate::AppState;

mod agents;
mod ask;
mod notify;
mod status;

pub async fn serve(state: AppState) -> anyhow::Result<()> {
    let port = state.config.port;

    let app = Router::new()
        .route("/agents/register", post(agents::register))
        .route("/agents/:id",      delete(agents::deregister))
        .route("/ask",             post(ask::ask))
        .route("/notify",          post(notify::notify))
        .route("/status",          get(status::status))
        .with_state(state);

    let addr = format!("127.0.0.1:{port}");
    tracing::info!("Callout listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
