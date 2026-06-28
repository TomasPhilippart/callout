use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use crate::AppState;
use super::AppError;

#[derive(Deserialize)]
pub struct NotifyRequest {
    pub agent_id: String,
    pub message: String,
}

pub async fn notify(
    State(state): State<AppState>,
    Json(req): Json<NotifyRequest>,
) -> Result<StatusCode, AppError> {
    if req.message.trim().is_empty() {
        return Err(AppError::BadRequest("message must not be empty".into()));
    }

    let name = {
        let mut agents = state.agents.write().await;
        agents.touch(&req.agent_id);
        agents.name(&req.agent_id)
    };

    tracing::info!(agent = %name, message = %req.message, "notify");

    let text = format!("{name}: {}", req.message);
    let _ = state.tts_tx.send(text).await;

    Ok(StatusCode::OK)
}
