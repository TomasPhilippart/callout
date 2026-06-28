use axum::{extract::State, Json};
use serde::Serialize;
use crate::AppState;

#[derive(Serialize)]
pub struct StatusResponse {
    pub agents: Vec<AgentStatus>,
}

#[derive(Serialize)]
pub struct AgentStatus {
    pub id: String,
    pub name: String,
    pub state: String,
    pub last_seen_secs: u64,
}

pub async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let agents = state.agents.read().await;
    let list = agents
        .all()
        .iter()
        .map(|a| AgentStatus {
            id: a.id.clone(),
            name: a.name.clone(),
            state: format!("{:?}", a.state).to_lowercase(),
            last_seen_secs: a.last_seen.elapsed().as_secs(),
        })
        .collect();
    Json(StatusResponse { agents: list })
}
