use serde::Deserialize;
use std::collections::HashMap;
use crate::Config;

#[derive(Debug, Deserialize, Default)]
pub struct Glossary {
    #[serde(default)]
    pub terms: Vec<String>,
    #[serde(default)]
    pub corrections: HashMap<String, String>,
}

impl Glossary {
    pub fn load() -> Self {
        let path = Config::dir().join("glossary.toml");
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path).and_then(|s| {
            toml::from_str(&s).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        }) {
            Ok(g) => g,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to load glossary, using empty defaults");
                Self::default()
            }
        }
    }

    pub fn whisper_prompt(&self, extra_terms: &[String]) -> String {
        let mut terms = self.terms.clone();
        terms.extend_from_slice(extra_terms);
        terms.join(", ")
    }

    pub fn apply_corrections(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (from, to) in &self.corrections {
            out = out.replace(from.as_str(), to.as_str());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glossary_with(terms: &[&str], corrections: &[(&str, &str)]) -> Glossary {
        Glossary {
            terms: terms.iter().map(|s| s.to_string()).collect(),
            corrections: corrections.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn whisper_prompt_merges_terms() {
        let g = glossary_with(&["Callout", "Axum"], &[]);
        let prompt = g.whisper_prompt(&["Tokio".to_string()]);
        assert!(prompt.contains("Callout"));
        assert!(prompt.contains("Axum"));
        assert!(prompt.contains("Tokio"));
    }

    #[test]
    fn apply_corrections_replaces_all() {
        let g = glossary_with(&[], &[("callout", "Callout"), ("axum", "Axum")]);
        let result = g.apply_corrections("i use callout and axum");
        assert_eq!(result, "i use Callout and Axum");
    }

    #[test]
    fn apply_corrections_noop_when_empty() {
        let g = Glossary::default();
        assert_eq!(g.apply_corrections("hello world"), "hello world");
    }
}
