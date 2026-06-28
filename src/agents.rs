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
        Self {
            agents: HashMap::new(),
        }
    }

    pub fn register(
        &mut self,
        name: String,
        description: Option<String>,
        context_terms: Vec<String>,
    ) -> String {
        let id = loop {
            let candidate = &Uuid::new_v4().to_string()[..6];
            if !self.agents.contains_key(candidate) {
                break candidate.to_string();
            }
        };
        self.agents.insert(
            id.clone(),
            Agent {
                id: id.clone(),
                name,
                description,
                context_terms,
                state: AgentState::Idle,
                last_seen: Instant::now(),
            },
        );
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

    pub fn prune_stale_after(&mut self, stale_secs: u64) {
        self.agents
            .retain(|_, a| a.last_seen.elapsed().as_secs() < stale_secs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_returns_6_char_id() {
        let mut reg = AgentRegistry::new();
        let id = reg.register("Claude".into(), None, vec![]);
        assert_eq!(id.len(), 6);
    }

    #[test]
    fn register_ids_are_unique() {
        let mut reg = AgentRegistry::new();
        let ids: Vec<_> = (0..100)
            .map(|i| reg.register(format!("agent-{i}"), None, vec![]))
            .collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 100);
    }

    #[test]
    fn remove_is_idempotent() {
        let mut reg = AgentRegistry::new();
        let id = reg.register("Test".into(), None, vec![]);
        assert!(reg.remove(&id));
        assert!(!reg.remove(&id));
    }

    #[test]
    fn name_falls_back_to_unknown() {
        let reg = AgentRegistry::new();
        assert_eq!(reg.name("nosuchid"), "unknown");
    }

    #[test]
    fn prune_stale_keeps_fresh_agents() {
        let mut reg = AgentRegistry::new();
        let id = reg.register("Fresh".into(), None, vec![]);
        reg.prune_stale_after(300);
        assert_eq!(reg.name(&id), "Fresh");
    }

    #[test]
    fn set_state_changes_agent_state() {
        let mut reg = AgentRegistry::new();
        let id = reg.register("Bot".into(), None, vec![]);
        reg.set_state(&id, AgentState::Waiting);
        let agent = reg.all().into_iter().find(|a| a.id == id).unwrap();
        assert_eq!(agent.state, AgentState::Waiting);
    }
}
