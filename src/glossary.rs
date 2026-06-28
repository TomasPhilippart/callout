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
        if path.exists() {
            if let Ok(s) = std::fs::read_to_string(&path) {
                if let Ok(g) = toml::from_str(&s) {
                    return g;
                }
            }
        }
        Self::default()
    }

    /// Build the initial_prompt string fed to Whisper.
    pub fn whisper_prompt(&self, extra_terms: &[String]) -> String {
        let mut terms = self.terms.clone();
        terms.extend_from_slice(extra_terms);
        terms.join(", ")
    }

    /// Apply hard find-and-replace corrections to a transcript.
    pub fn apply_corrections(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (from, to) in &self.corrections {
            out = out.replace(from.as_str(), to.as_str());
        }
        out
    }
}
