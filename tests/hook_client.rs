use callout::{
    agents::AgentRegistry, api, glossary::Glossary, recorder::Recorder, router::AskRouter,
    AppState, Config,
};
use std::sync::{atomic::AtomicBool, Arc};
use tokio::sync::{mpsc, Mutex, Notify, RwLock};

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

/// Boots a real callout API server on an ephemeral localhost port.
/// Returns the base URL (e.g. "http://127.0.0.1:54321").
async fn spawn_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = api::build_app(test_state());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn register_agent_returns_id() {
    let base = spawn_server().await;
    let id =
        tokio::task::spawn_blocking(move || callout::hook::register_agent(&base, "Test Agent"))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(id.len(), 6);
}

#[tokio::test]
async fn notify_succeeds() {
    let base = spawn_server().await;
    let id = tokio::task::spawn_blocking({
        let base = base.clone();
        move || callout::hook::register_agent(&base, "Test Agent")
    })
    .await
    .unwrap()
    .unwrap();

    tokio::task::spawn_blocking(move || callout::hook::notify(&base, &id, "hello"))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn status_lists_registered_agent() {
    let base = spawn_server().await;
    let id = tokio::task::spawn_blocking({
        let base = base.clone();
        move || callout::hook::register_agent(&base, "Test Agent")
    })
    .await
    .unwrap()
    .unwrap();

    let ids = tokio::task::spawn_blocking(move || callout::hook::status_agent_ids(&base))
        .await
        .unwrap()
        .unwrap();
    assert!(ids.contains(&id));
}
