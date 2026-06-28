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
        Self {
            pending: HashMap::new(),
        }
    }

    pub fn insert(&mut self, agent_id: String, ask: PendingAsk) {
        self.pending.insert(agent_id, ask);
    }

    pub fn remove(&mut self, agent_id: &str) -> Option<PendingAsk> {
        self.pending.remove(agent_id)
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
            let _ = ask.tx.send(AskResponse {
                answer,
                raw: Some(raw),
            });
            true
        } else {
            false
        }
    }

    pub fn pending_agent_ids(&self) -> impl Iterator<Item = &str> {
        self.pending.keys().map(String::as_str)
    }
}

fn match_choice<'a>(choices: &'a [Choice], transcript: &str) -> Option<&'a str> {
    let t = transcript.to_lowercase();

    for c in choices {
        if t.contains(&c.key.to_lowercase()) || t.contains(&c.label.to_lowercase()) {
            return Some(&c.key);
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ask(choices: Vec<Choice>) -> (PendingAsk, oneshot::Receiver<AskResponse>) {
        let (tx, rx) = oneshot::channel();
        let ask = PendingAsk {
            agent_id: "agent1".into(),
            question: "Proceed?".into(),
            choices,
            tx,
        };
        (ask, rx)
    }

    #[test]
    fn resolve_no_choices_returns_raw() {
        let mut router = AskRouter::new();
        let (ask, mut rx) = make_ask(vec![]);
        router.insert("agent1".into(), ask);

        assert!(router.resolve("agent1", "hello world".into()));
        let resp = rx.try_recv().unwrap();
        assert_eq!(resp.answer.as_deref(), Some("hello world"));
        assert_eq!(resp.raw.as_deref(), Some("hello world"));
    }

    #[test]
    fn resolve_exact_key_match() {
        let mut router = AskRouter::new();
        let (ask, mut rx) = make_ask(vec![
            Choice {
                key: "yes".into(),
                label: "Yes, proceed".into(),
            },
            Choice {
                key: "no".into(),
                label: "No, cancel".into(),
            },
        ]);
        router.insert("agent1".into(), ask);

        router.resolve("agent1", "yes please do it".into());
        let resp = rx.try_recv().unwrap();
        assert_eq!(resp.answer.as_deref(), Some("yes"));
    }

    #[test]
    fn resolve_exact_label_match() {
        let mut router = AskRouter::new();
        let (ask, mut rx) = make_ask(vec![
            Choice {
                key: "y".into(),
                label: "confirm".into(),
            },
            Choice {
                key: "n".into(),
                label: "deny".into(),
            },
        ]);
        router.insert("agent1".into(), ask);

        router.resolve("agent1", "I want to confirm that".into());
        let resp = rx.try_recv().unwrap();
        assert_eq!(resp.answer.as_deref(), Some("y"));
    }

    #[test]
    fn resolve_unknown_agent_returns_false() {
        let mut router = AskRouter::new();
        assert!(!router.resolve("nobody", "hello".into()));
    }

    #[test]
    fn resolve_removes_pending_ask() {
        let mut router = AskRouter::new();
        let (ask, _rx) = make_ask(vec![]);
        router.insert("agent1".into(), ask);
        router.resolve("agent1", "done".into());
        assert!(!router.resolve("agent1", "again".into()));
    }
}
