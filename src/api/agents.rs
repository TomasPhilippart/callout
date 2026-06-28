use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use crate::AppState;

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
) -> Json<RegisterResponse> {
    let id = state.agents.write().await.register(
        req.name,
        req.description,
        req.context_terms,
    );
    tracing::info!("Agent registered: {id}");
    Json(RegisterResponse { agent_id: id })
}

pub async fn deregister(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> StatusCode {
    if state.agents.write().await.remove(&id) {
        tracing::info!("Agent deregistered: {id}");
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}
