use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};
use crate::{
    agents::AgentState,
    router::{AskResponse, Choice, PendingAsk},
    AppState,
};
use super::AppError;

#[derive(Deserialize)]
pub struct AskRequest {
    pub agent_id: String,
    pub question: String,
    #[serde(default)]
    pub choices: Vec<ChoiceInput>,
    pub timeout_seconds: Option<u64>,
    pub default: Option<String>,
}

#[derive(Deserialize)]
pub struct ChoiceInput {
    pub key: String,
    pub label: String,
}

#[derive(Serialize)]
pub struct AskResponseBody {
    pub answer: Option<String>,
    pub answers: Vec<String>,
    pub raw: Option<String>,
    pub timed_out: bool,
}

pub async fn ask(
    State(state): State<AppState>,
    Json(req): Json<AskRequest>,
) -> Result<Json<AskResponseBody>, AppError> {
    if req.question.trim().is_empty() {
        return Err(AppError::BadRequest("question must not be empty".into()));
    }
    let timeout_secs = req.timeout_seconds.unwrap_or(120);
    if !(1..=3600).contains(&timeout_secs) {
        return Err(AppError::BadRequest(
            "timeout_seconds must be between 1 and 3600".into(),
        ));
    }

    let name = {
        let mut agents = state.agents.write().await;
        agents.touch(&req.agent_id);
        agents.set_state(&req.agent_id, AgentState::Waiting);
        agents.name(&req.agent_id)
    };

    tracing::info!(agent = %name, question = %req.question, "ask");

    let spoken = build_spoken_prompt(&name, &req.question, &req.choices);
    let voice = state.config.tts.voice.clone();
    tokio::spawn(async move {
        crate::speaker::speak(&spoken, &voice).await;
    });

    let (tx, rx) = oneshot::channel::<AskResponse>();
    state.router.lock().await.insert(
        req.agent_id.clone(),
        PendingAsk {
            agent_id: req.agent_id.clone(),
            question: req.question.clone(),
            choices: req.choices.iter()
                .map(|c| Choice { key: c.key.clone(), label: c.label.clone() })
                .collect(),
            tx,
        },
    );

    let result = timeout(Duration::from_secs(timeout_secs), rx).await;

    state.agents.write().await.set_state(&req.agent_id, AgentState::Idle);
    state.router.lock().await.remove(&req.agent_id);

    match result {
        Ok(Ok(resp)) => {
            tracing::info!(agent = %name, answer = ?resp.answer, "ask resolved");
            let answers = resp.answer.clone().into_iter().collect();
            Ok(Json(AskResponseBody {
                answer: resp.answer,
                answers,
                raw: resp.raw,
                timed_out: false,
            }))
        }
        _ => {
            tracing::warn!(agent = %name, default = ?req.default, "ask timed out");
            let answers = req.default.clone().into_iter().collect();
            Ok(Json(AskResponseBody {
                answer: req.default,
                answers,
                raw: None,
                timed_out: true,
            }))
        }
    }
}

fn build_spoken_prompt(name: &str, question: &str, choices: &[ChoiceInput]) -> String {
    let mut s = format!("{name} asks: {question}");
    if !choices.is_empty() {
        let opts: Vec<String> = choices
            .iter()
            .map(|c| format!("{}: {}", c.key, c.label))
            .collect();
        s.push_str(&format!(". {}", opts.join(". ")));
    }
    s
}
