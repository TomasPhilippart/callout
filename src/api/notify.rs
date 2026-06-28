use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use crate::AppState;

#[derive(Deserialize)]
pub struct NotifyRequest {
    pub agent_id: String,
    pub message: String,
}

pub async fn notify(
    State(state): State<AppState>,
    Json(req): Json<NotifyRequest>,
) -> StatusCode {
    let name = {
        let mut agents = state.agents.write().await;
        agents.touch(&req.agent_id);
        agents.name(&req.agent_id)
    };

    tracing::info!("[{name}] {}", req.message);

    let text = format!("{name}: {}", req.message);
    tokio::spawn(async move {
        crate::speaker::speak(&text).await;
    });

    StatusCode::OK
}
