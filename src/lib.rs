use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};

pub mod agents;
pub mod api;
pub mod cli;
pub mod config;
pub mod glossary;
pub mod router;
pub mod speaker;
pub mod voices;

pub use config::Config;

#[derive(Clone)]
pub struct AppState {
    pub agents:   Arc<RwLock<agents::AgentRegistry>>,
    pub router:   Arc<Mutex<router::AskRouter>>,
    pub config:   Arc<Config>,
    pub glossary: Arc<glossary::Glossary>,
    /// Send text here to queue it for serial TTS playback.
    pub tts_tx:   mpsc::Sender<String>,
}

pub async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "callout=info".into()),
        )
        .init();

    let config = Config::load()?;
    let glossary = glossary::Glossary::load();

    let (tts_tx, mut tts_rx) = mpsc::channel::<String>(32);
    let tts_voice = config.tts.voice.clone();

    tokio::spawn(async move {
        while let Some(text) = tts_rx.recv().await {
            speaker::speak(&text, &tts_voice).await;
        }
    });

    let state = AppState {
        agents:   Arc::new(RwLock::new(agents::AgentRegistry::new())),
        router:   Arc::new(Mutex::new(router::AskRouter::new())),
        config:   Arc::new(config),
        glossary: Arc::new(glossary),
        tts_tx,
    };

    {
        let agents = state.agents.clone();
        let prune_interval = state.config.daemon.prune_interval_secs;
        let stale_secs = state.config.daemon.stale_secs;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                tokio::time::Duration::from_secs(prune_interval)
            );
            loop {
                interval.tick().await;
                agents.write().await.prune_stale_after(stale_secs);
            }
        });
    }

    let configured_voice = &state.config.tts.voice;
    if voices::is_installed(configured_voice) {
        tracing::info!(voice = %configured_voice, "TTS voice ready");
    } else {
        tracing::warn!(
            voice = %configured_voice,
            "configured voice not installed — run 'callout voices list' to see what's available"
        );
    }

    api::serve(state).await
}
