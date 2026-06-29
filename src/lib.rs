use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};

pub mod agents;
pub mod api;
pub mod cli;
pub mod config;
pub mod glossary;
pub mod model;
pub mod ptt;
pub mod recorder;
pub mod router;
pub mod speaker;
pub mod transcriber;
pub mod voices;

pub use config::Config;

#[derive(Clone)]
pub struct AppState {
    pub agents: Arc<RwLock<agents::AgentRegistry>>,
    pub router: Arc<Mutex<router::AskRouter>>,
    pub config: Arc<Config>,
    pub glossary: Arc<glossary::Glossary>,
    pub tts_tx: mpsc::Sender<String>,
    /// Shared recorder for HTTP-triggered PTT (/ptt/start, /ptt/stop)
    pub ptt_recorder: Arc<Mutex<recorder::Recorder>>,
    /// Whisper transcriber — None if model not loaded
    pub transcriber: Option<Arc<transcriber::Transcriber>>,
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

    // Load Whisper model if available
    let model_path = Config::model_path(&config.model);
    let transcriber: Option<Arc<transcriber::Transcriber>> = if model_path.exists() {
        match transcriber::Transcriber::load(&model_path) {
            Ok(t) => {
                tracing::info!(model = %config.model, "Whisper model loaded");
                Some(Arc::new(t))
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load Whisper model — PTT disabled");
                None
            }
        }
    } else {
        tracing::warn!(
            path = %model_path.display(),
            "Whisper model not found — PTT disabled. Run 'callout model download' to get it."
        );
        None
    };

    let state = AppState {
        agents: Arc::new(RwLock::new(agents::AgentRegistry::new())),
        router: Arc::new(Mutex::new(router::AskRouter::new())),
        config: Arc::new(config),
        glossary: Arc::new(glossary),
        tts_tx,
        ptt_recorder: Arc::new(Mutex::new(recorder::Recorder::default())),
        transcriber,
    };

    // Agent stale-pruning task
    {
        let agents = state.agents.clone();
        let prune_interval = state.config.daemon.prune_interval_secs;
        let stale_secs = state.config.daemon.stale_secs;
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(prune_interval));
            loop {
                interval.tick().await;
                agents.write().await.prune_stale_after(stale_secs);
            }
        });
    }

    // TTS voice check
    let configured_voice = &state.config.tts.voice;
    if voices::is_installed(configured_voice) {
        tracing::info!(voice = %configured_voice, "TTS voice ready");
    } else {
        tracing::warn!(
            voice = %configured_voice,
            "configured voice not installed — run 'callout voices list'"
        );
    }

    if let Some(t) = state.transcriber.clone() {
        ptt::spawn(state.clone(), t);
    }

    api::serve(state).await
}
