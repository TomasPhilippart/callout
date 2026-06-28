use serde::Serialize;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub context_terms: Vec<String>,
    pub state: AgentState,
    #[serde(skip)]
    pub last_seen: Instant,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Idle,
    Waiting,
}

pub struct AgentRegistry {
    agents: HashMap<String, Agent>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self { agents: HashMap::new() }
    }

    pub fn register(
        &mut self,
        name: String,
        description: Option<String>,
        context_terms: Vec<String>,
    ) -> String {
        let id = Uuid::new_v4().to_string()[..6].to_string();
        self.agents.insert(id.clone(), Agent {
            id: id.clone(),
            name,
            description,
            context_terms,
            state: AgentState::Idle,
            last_seen: Instant::now(),
        });
        id
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.agents.remove(id).is_some()
    }

    pub fn touch(&mut self, id: &str) {
        if let Some(a) = self.agents.get_mut(id) {
            a.last_seen = Instant::now();
        }
    }

    pub fn set_state(&mut self, id: &str, state: AgentState) {
        if let Some(a) = self.agents.get_mut(id) {
            a.state = state;
        }
    }

    pub fn name(&self, id: &str) -> String {
        self.agents
            .get(id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "unknown".into())
    }

    pub fn context_terms(&self, id: &str) -> Vec<String> {
        self.agents
            .get(id)
            .map(|a| a.context_terms.clone())
            .unwrap_or_default()
    }

    pub fn all(&self) -> Vec<&Agent> {
        let mut list: Vec<_> = self.agents.values().collect();
        list.sort_by_key(|a| &a.id);
        list
    }

    pub fn prune_stale(&mut self) {
        self.agents
            .retain(|_, a| a.last_seen.elapsed().as_secs() < 300);
    }
}
