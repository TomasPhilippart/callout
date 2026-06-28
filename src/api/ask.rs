use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};
use crate::{
    agents::AgentState,
    router::{AskResponse, Choice, PendingAsk},
    AppState,
};

#[derive(Deserialize)]
pub struct AskRequest {
    pub agent_id: String,
    pub question: String,
    #[serde(default)]
    pub choices: Vec<ChoiceInput>,
    #[serde(default)]
    pub multi_select: bool,
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
) -> (StatusCode, Json<AskResponseBody>) {
    let timeout_secs = req.timeout_seconds.unwrap_or(120);
    let default_answer = req.default.clone();

    // Update agent state and grab its name
    let name = {
        let mut agents = state.agents.write().await;
        agents.touch(&req.agent_id);
        agents.set_state(&req.agent_id, AgentState::Waiting);
        agents.name(&req.agent_id)
    };

    // Build and speak the question
    let spoken = build_spoken_prompt(&name, &req.question, &req.choices);
    tracing::info!("ASK [{name}] {}", req.question);
    tokio::spawn(async move {
        crate::speaker::speak(&spoken).await;
    });

    // Register in router and wait
    let (tx, rx) = oneshot::channel::<AskResponse>();
    {
        let mut router = state.router.lock().await;
        router.insert(
            req.agent_id.clone(),
            PendingAsk {
                agent_id: req.agent_id.clone(),
                question: req.question.clone(),
                choices: req.choices.iter().map(|c| Choice { key: c.key.clone(), label: c.label.clone() }).collect(),
                tx,
            },
        );
    }

    let result = timeout(Duration::from_secs(timeout_secs), rx).await;

    // Clean up regardless of outcome
    {
        state.agents.write().await.set_state(&req.agent_id, AgentState::Idle);
        state.router.lock().await.remove(&req.agent_id);
    }

    match result {
        Ok(Ok(resp)) => {
            let answers = resp.answer.clone().into_iter().collect();
            (StatusCode::OK, Json(AskResponseBody {
                answer: resp.answer,
                answers,
                raw: resp.raw,
                timed_out: false,
            }))
        }
        _ => {
            tracing::warn!("ASK [{name}] timed out, using default: {:?}", default_answer);
            let answers = default_answer.clone().into_iter().collect();
            (StatusCode::OK, Json(AskResponseBody {
                answer: default_answer,
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
        let opts: Vec<String> = choices.iter().map(|c| format!("{}: {}", c.key, c.label)).collect();
        s.push_str(&format!(". {}", opts.join(". ")));
    }
    s
}
