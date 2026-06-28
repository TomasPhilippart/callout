use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

mod agents;
mod api;
mod config;
mod glossary;
mod router;
mod speaker;

pub use config::Config;

#[derive(Clone)]
pub struct AppState {
    pub agents: Arc<RwLock<agents::AgentRegistry>>,
    pub router: Arc<Mutex<router::AskRouter>>,
    pub config: Arc<Config>,
    pub glossary: Arc<glossary::Glossary>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    // Periodically prune agents not seen in 5 minutes
    {
        let agents = state.agents.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                agents.write().await.prune_stale();
            }
        });
    }

    api::serve(state).await
}
