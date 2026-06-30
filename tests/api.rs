use axum::body::Body;
use axum::http::{Request, StatusCode};
use callout::{
    agents::AgentRegistry, api, glossary::Glossary, recorder::Recorder, router::AskRouter,
    AppState, Config,
};
use serde_json::{json, Value};
use std::sync::{atomic::AtomicBool, Arc};
use tokio::sync::{mpsc, Mutex, Notify, RwLock};
use tower::ServiceExt;

fn test_state() -> AppState {
    let (tts_tx, _tts_rx) = mpsc::channel(8);
    AppState {
        agents: Arc::new(RwLock::new(AgentRegistry::new())),
        router: Arc::new(Mutex::new(AskRouter::new())),
        config: Arc::new(Config::default()),
        glossary: Arc::new(Glossary::default()),
        tts_tx,
        ptt_recorder: Arc::new(Mutex::new(Recorder::default())),
        transcriber: None,
        recording: Arc::new(AtomicBool::new(false)),
        tts_speaking: Arc::new(AtomicBool::new(false)),
        just_processed: Arc::new(AtomicBool::new(false)),
        tts_kill: Arc::new(Notify::new()),
        active_agent: Arc::new(std::sync::Mutex::new(None)),
    }
}

async fn request(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

// ── /agents/register ────────────────────────────────────────────────────────

#[tokio::test]
async fn register_returns_6_char_agent_id() {
    let (status, body) = request(
        api::build_app(test_state()),
        "POST",
        "/agents/register",
        Some(json!({"name": "Claude"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = body["agent_id"].as_str().unwrap();
    assert_eq!(id.len(), 6);
}

#[tokio::test]
async fn register_empty_name_is_400() {
    let (status, body) = request(
        api::build_app(test_state()),
        "POST",
        "/agents/register",
        Some(json!({"name": "  "})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn register_missing_name_is_422() {
    let (status, _) = request(
        api::build_app(test_state()),
        "POST",
        "/agents/register",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

// ── /agents/:id ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn deregister_existing_agent_is_204() {
    let state = test_state();
    let (_, reg) = request(
        api::build_app(state.clone()),
        "POST",
        "/agents/register",
        Some(json!({"name": "Bot"})),
    )
    .await;
    let id = reg["agent_id"].as_str().unwrap();

    let (status, _) = request(
        api::build_app(state),
        "DELETE",
        &format!("/agents/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn deregister_unknown_agent_is_404() {
    let (status, body) = request(
        api::build_app(test_state()),
        "DELETE",
        "/agents/zzzzzz",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].is_string());
}

// ── /status ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn status_returns_registered_agents() {
    let state = test_state();
    let (_, reg) = request(
        api::build_app(state.clone()),
        "POST",
        "/agents/register",
        Some(json!({"name": "StatusBot"})),
    )
    .await;
    let id = reg["agent_id"].as_str().unwrap();

    let (status, body) = request(api::build_app(state), "GET", "/status", None).await;
    assert_eq!(status, StatusCode::OK);
    let agents = body["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["id"].as_str().unwrap(), id);
    assert_eq!(agents[0]["name"].as_str().unwrap(), "StatusBot");
    assert_eq!(agents[0]["state"].as_str().unwrap(), "idle");
}

#[tokio::test]
async fn status_empty_when_no_agents() {
    let (status, body) = request(api::build_app(test_state()), "GET", "/status", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["agents"].as_array().unwrap().len(), 0);
}

// ── /notify ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn notify_empty_message_is_400() {
    let (status, body) = request(
        api::build_app(test_state()),
        "POST",
        "/notify",
        Some(json!({"agent_id": "abc123", "message": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn notify_unknown_agent_succeeds_as_unknown() {
    let (status, _) = request(
        api::build_app(test_state()),
        "POST",
        "/notify",
        Some(json!({"agent_id": "nobody", "message": "hello"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

// ── /ask ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ask_empty_question_is_400() {
    let (status, body) = request(
        api::build_app(test_state()),
        "POST",
        "/ask",
        Some(json!({"agent_id": "abc123", "question": "  "})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn ask_invalid_timeout_is_400() {
    let (status, _) = request(
        api::build_app(test_state()),
        "POST",
        "/ask",
        Some(json!({"agent_id": "abc123", "question": "Ready?", "timeout_seconds": 9999})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ask_times_out_and_returns_default() {
    let (status, body) = request(
        api::build_app(test_state()),
        "POST",
        "/ask",
        Some(json!({
            "agent_id": "abc123",
            "question": "Shall we proceed?",
            "timeout_seconds": 1,
            "default": "yes"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["timed_out"].as_bool(), Some(true));
    assert_eq!(body["answer"].as_str(), Some("yes"));
}
