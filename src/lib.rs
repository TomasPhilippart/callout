use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

pub mod agents;
pub mod api;
pub mod config;
pub mod glossary;
pub mod router;
pub mod speaker;

pub use config::Config;

#[derive(Clone)]
pub struct AppState {
    pub agents: Arc<RwLock<agents::AgentRegistry>>,
    pub router: Arc<Mutex<router::AskRouter>>,
    pub config: Arc<Config>,
    pub glossary: Arc<glossary::Glossary>,
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

    let state = AppState {
        agents: Arc::new(RwLock::new(agents::AgentRegistry::new())),
        router: Arc::new(Mutex::new(router::AskRouter::new())),
        config: Arc::new(config),
        glossary: Arc::new(glossary),
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

    tracing::info!(voice = %state.config.tts.voice, "TTS voice configured");
    api::serve(state).await
}
