use std::collections::HashMap;
use tokio::sync::oneshot;

#[derive(Debug, Clone)]
pub struct Choice {
    pub key: String,
    pub label: String,
}

pub struct AskResponse {
    pub answer: Option<String>,
    pub raw: Option<String>,
}

pub struct PendingAsk {
    pub agent_id: String,
    pub question: String,
    pub choices: Vec<Choice>,
    pub tx: oneshot::Sender<AskResponse>,
}

pub struct AskRouter {
    pending: HashMap<String, PendingAsk>,
}

impl AskRouter {
    pub fn new() -> Self {
        Self { pending: HashMap::new() }
    }

    pub fn insert(&mut self, agent_id: String, ask: PendingAsk) {
        self.pending.insert(agent_id, ask);
    }

    pub fn remove(&mut self, agent_id: &str) -> Option<PendingAsk> {
        self.pending.remove(agent_id)
    }

    /// Returns the agent_id of the first waiting ask, if any.
    pub fn first_pending_id(&self) -> Option<String> {
        self.pending.keys().next().cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Resolve a pending ask by matching the transcript against choices.
    /// Falls back to raw transcript if no choices or no match found.
    pub fn resolve(&mut self, agent_id: &str, raw: String) -> bool {
        if let Some(ask) = self.pending.remove(agent_id) {
            let answer = if ask.choices.is_empty() {
                Some(raw.clone())
            } else {
                match_choice(&ask.choices, &raw)
                    .map(|k| k.to_string())
                    .or(Some(raw.clone()))
            };
            let _ = ask.tx.send(AskResponse { answer, raw: Some(raw) });
            true
        } else {
            false
        }
    }
}

/// Fuzzy-match a transcript against available choice labels/keys.
fn match_choice<'a>(choices: &'a [Choice], transcript: &str) -> Option<&'a str> {
    let t = transcript.to_lowercase();

    // Exact key match first ("a", "b", "yes", "no")
    for c in choices {
        if t.contains(&c.key.to_lowercase()) || t.contains(&c.label.to_lowercase()) {
            return Some(&c.key);
        }
    }

    // Fuzzy fallback via strsim
    choices
        .iter()
        .map(|c| {
            let score = strsim::jaro_winkler(&t, &c.label.to_lowercase())
                .max(strsim::jaro_winkler(&t, &c.key.to_lowercase()));
            (score, c.key.as_str())
        })
        .filter(|(s, _)| *s > 0.8)
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
        .map(|(_, k)| k)
}
