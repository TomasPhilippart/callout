use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub agent_id: String,
}

pub type Registry = HashMap<String, SessionEntry>;

pub fn load_registry(path: &Path) -> Registry {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_registry(path: &Path, registry: &Registry) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let s = serde_json::to_string_pretty(registry)?;
    std::fs::write(path, s).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_registry_missing_file_returns_empty() {
        let path = std::env::temp_dir().join("callout-test-missing-sessions.json");
        let _ = std::fs::remove_file(&path);
        assert!(load_registry(&path).is_empty());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let path = std::env::temp_dir().join("callout-test-roundtrip-sessions.json");
        let mut registry = Registry::new();
        registry.insert(
            "sess1".into(),
            SessionEntry {
                agent_id: "abc123".into(),
            },
        );
        save_registry(&path, &registry).unwrap();
        let loaded = load_registry(&path);
        assert_eq!(loaded.get("sess1").unwrap().agent_id, "abc123");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_creates_parent_dir() {
        let dir = std::env::temp_dir().join(format!("callout-test-dir-{}", std::process::id()));
        let path = dir.join("sessions.json");
        let _ = std::fs::remove_dir_all(&dir);
        save_registry(&path, &Registry::new()).unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
