use std::sync::{atomic::AtomicBool, Arc};
use tokio::sync::{mpsc, Mutex, Notify, RwLock};

pub mod agents;
pub mod api;
pub mod cli;
pub mod config;
pub mod glossary;
#[cfg(target_os = "macos")]
pub mod install;
pub mod model;
pub mod ptt;
pub mod recorder;
pub mod router;
pub mod speaker;
pub mod transcriber;
#[cfg(target_os = "macos")]
pub mod tray;
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
    /// True while the PTT thread is actively capturing audio
    pub recording: Arc<AtomicBool>,
    /// True while say(1) is speaking
    pub tts_speaking: Arc<AtomicBool>,
    /// Pulses true for one tray-update cycle after a successful transcription
    pub just_processed: Arc<AtomicBool>,
    /// Signal the TTS task to interrupt the current `say` process (barge-in)
    pub tts_kill: Arc<Notify>,
    /// Pre-selected agent for the next PTT press (set from tray, consumed by ptt.rs)
    pub active_agent: Arc<std::sync::Mutex<Option<String>>>,
}

pub async fn run() -> anyhow::Result<()> {
    run_inner(None).await
}

/// Like `run()` but sends a clone of AppState once it is ready.
/// Used by the macOS main thread to read live agent/recording state for the tray.
pub async fn run_with(tx: std::sync::mpsc::SyncSender<AppState>) -> anyhow::Result<()> {
    run_inner(Some(tx)).await
}

async fn run_inner(state_tx: Option<std::sync::mpsc::SyncSender<AppState>>) -> anyhow::Result<()> {
    use tracing_subscriber::prelude::*;

    std::fs::create_dir_all(Config::logs_dir()).ok();
    let file_appender = tracing_appender::rolling::daily(Config::logs_dir(), "callout.log");
    let (file_writer, _log_guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "callout=info".into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(file_writer)
                .with_ansi(false),
        )
        .init();

    let config = Config::load()?;
    let glossary = glossary::Glossary::load();

    let (tts_tx, mut tts_rx) = mpsc::channel::<String>(32);
    let tts_voice = config.tts.voice.clone();
    let tts_kill = Arc::new(Notify::new());
    let tts_speaking: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    {
        let tts_kill = tts_kill.clone();
        let tts_speaking = tts_speaking.clone();
        tokio::spawn(async move {
            while let Some(text) = tts_rx.recv().await {
                speaker::speak(&text, &tts_voice, &tts_kill, &tts_speaking).await;
            }
        });
    }

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
        recording: Arc::new(AtomicBool::new(false)),
        tts_speaking,
        just_processed: Arc::new(AtomicBool::new(false)),
        tts_kill,
        active_agent: Arc::new(std::sync::Mutex::new(None)),
    };

    // Send AppState to the main thread (for tray updates) before serving
    if let Some(tx) = state_tx {
        tx.send(state.clone()).ok();
    }

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
