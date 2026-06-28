use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use crate::AppState;
use super::AppError;

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub context_terms: Vec<String>,
}

#[derive(Serialize)]
pub struct RegisterResponse {
    pub agent_id: String,
}

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, AppError> {
    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("name must not be empty".into()));
    }

    let id = state.agents.write().await.register(
        req.name.trim().to_string(),
        req.description,
        req.context_terms,
    );
    tracing::info!(agent_id = %id, "agent registered");
    Ok(Json(RegisterResponse { agent_id: id }))
}

pub async fn deregister(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    if state.agents.write().await.remove(&id) {
        tracing::info!(agent_id = %id, "agent deregistered");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound(format!("agent {id} not found")))
    }
}
