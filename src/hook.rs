use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub agent_id: String,
}

// Not safe for concurrent writers — revisit if Task 3+ shows this is a real problem.
pub type SessionRegistry = HashMap<String, SessionEntry>;

pub fn load_registry(path: &Path) -> SessionRegistry {
    let Ok(s) = std::fs::read_to_string(path) else {
        return SessionRegistry::default();
    };
    match serde_json::from_str(&s) {
        Ok(registry) => registry,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "failed to parse session registry, using empty defaults"
            );
            SessionRegistry::default()
        }
    }
}

// Not safe for concurrent writers — revisit if Task 3+ shows this is a real problem.
pub fn save_registry(path: &Path, registry: &SessionRegistry) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let s =
        serde_json::to_string_pretty(registry).context("failed to serialize session registry")?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, s)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            path.display()
        )
    })
}

fn http_error(context: &str, e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Status(status, response) => {
            let body = response
                .into_string()
                .unwrap_or_else(|_| "<failed to read response body>".to_string());
            anyhow::anyhow!("{context}: HTTP {status}: {body}")
        }
        transport_err @ ureq::Error::Transport(_) => {
            anyhow::anyhow!("{context}: {transport_err}")
        }
    }
}

pub fn register_agent(base_url: &str, name: &str) -> Result<String> {
    #[derive(Serialize)]
    struct Req<'a> {
        name: &'a str,
    }
    #[derive(Deserialize)]
    struct Resp {
        agent_id: String,
    }
    let resp: Resp = ureq::post(&format!("{base_url}/agents/register"))
        .send_json(Req { name })
        .map_err(|e| http_error("agent registration failed", e))?
        .into_json()
        .context("failed to parse register response")?;
    Ok(resp.agent_id)
}

pub fn notify(base_url: &str, agent_id: &str, message: &str) -> Result<()> {
    #[derive(Serialize)]
    struct Req<'a> {
        agent_id: &'a str,
        message: &'a str,
    }
    ureq::post(&format!("{base_url}/notify"))
        .send_json(Req { agent_id, message })
        .map_err(|e| http_error("notify failed", e))?;
    Ok(())
}

/// Deliberately only captures `answer`/`timed_out` from the server's
/// `AskResponseBody` — `answers`/`raw` are not needed by callers of this
/// client and are intentionally dropped, not overlooked.
#[derive(Deserialize)]
pub struct AskResult {
    pub answer: Option<String>,
    pub timed_out: bool,
}

pub fn ask(
    base_url: &str,
    agent_id: &str,
    question: &str,
    choices: &[(String, String)],
    timeout_seconds: u64,
    default: Option<&str>,
) -> Result<AskResult> {
    #[derive(Serialize)]
    struct ChoiceReq<'a> {
        key: &'a str,
        label: &'a str,
    }
    #[derive(Serialize)]
    struct Req<'a> {
        agent_id: &'a str,
        question: &'a str,
        choices: Vec<ChoiceReq<'a>>,
        timeout_seconds: u64,
        default: Option<&'a str>,
    }
    let req = Req {
        agent_id,
        question,
        choices: choices
            .iter()
            .map(|(k, l)| ChoiceReq { key: k, label: l })
            .collect(),
        timeout_seconds,
        default,
    };
    let result: AskResult = ureq::post(&format!("{base_url}/ask"))
        .send_json(req)
        .map_err(|e| http_error("ask failed", e))?
        .into_json()
        .context("failed to parse ask response")?;
    Ok(result)
}

// Not safe for concurrent callers on the same session_id — see SessionRegistry's concurrency note.
pub fn resolve_agent_id(
    base_url: &str,
    registry_path: &Path,
    session_id: &str,
    name_hint: &str,
) -> Result<String> {
    let mut registry = load_registry(registry_path);

    if let Some(entry) = registry.get(session_id) {
        let known_ids = status_agent_ids(base_url)?;
        if known_ids.contains(&entry.agent_id) {
            return Ok(entry.agent_id.clone());
        }
        tracing::info!(
            session_id,
            stale_agent_id = %entry.agent_id,
            "cached agent_id is stale — re-registering"
        );
    }

    let agent_id = register_agent(base_url, name_hint)?;
    registry.insert(
        session_id.to_string(),
        SessionEntry {
            agent_id: agent_id.clone(),
        },
    );
    save_registry(registry_path, &registry)?;
    Ok(agent_id)
}

pub fn status_agent_ids(base_url: &str) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct AgentStatus {
        id: String,
    }
    #[derive(Deserialize)]
    struct StatusResp {
        agents: Vec<AgentStatus>,
    }
    let resp: StatusResp = ureq::get(&format!("{base_url}/status"))
        .call()
        .map_err(|e| http_error("status check failed", e))?
        .into_json()
        .context("failed to parse status response")?;
    Ok(resp.agents.into_iter().map(|a| a.id).collect())
}

pub fn pretooluse_question(
    agent_name: &str,
    tool_name: &str,
    tool_input: &serde_json::Value,
) -> (String, Vec<(String, String)>) {
    let detail = tool_input
        .get("command")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| tool_input.to_string());
    let question = format!("{agent_name} wants to run {tool_name}: {detail}. Allow?");
    (
        question,
        vec![
            ("y".to_string(), "yes".to_string()),
            ("n".to_string(), "no".to_string()),
        ],
    )
}

/// Maps a PTT answer to Claude Code's PreToolUse decision JSON.
/// `None` (timeout, or no answer) fails safe to deny.
pub fn decision_json(answer: Option<&str>) -> serde_json::Value {
    let allow = matches!(
        answer.map(str::to_lowercase).as_deref(),
        Some("y") | Some("yes")
    );
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": if allow { "allow" } else { "deny" },
            "permissionDecisionReason": if allow { "approved via voice" } else { "denied via voice (or no response)" }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn load_registry_missing_file_returns_empty() {
        let path = std::env::temp_dir().join(format!(
            "callout-test-missing-sessions-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        assert!(load_registry(&path).is_empty());
    }

    #[test]
    fn load_registry_corrupt_file_returns_empty_and_logs() {
        let path = std::env::temp_dir().join(format!(
            "callout-test-corrupt-sessions-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, "not valid json").unwrap();
        assert!(load_registry(&path).is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let path = std::env::temp_dir().join(format!(
            "callout-test-roundtrip-sessions-{}.json",
            std::process::id()
        ));
        let mut registry = SessionRegistry::new();
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
        save_registry(&path, &SessionRegistry::new()).unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pretooluse_question_includes_command_for_bash() {
        let (q, choices) =
            pretooluse_question("Test Agent", "Bash", &json!({"command": "rm -rf dist"}));
        assert!(q.contains("Test Agent"));
        assert!(q.contains("rm -rf dist"));
        assert_eq!(
            choices,
            vec![
                ("y".to_string(), "yes".to_string()),
                ("n".to_string(), "no".to_string())
            ]
        );
    }

    #[test]
    fn decision_json_approve_on_yes() {
        let v = decision_json(Some("y"));
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
    }

    #[test]
    fn decision_json_deny_on_no() {
        let v = decision_json(Some("n"));
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    }

    #[test]
    fn decision_json_deny_on_timeout_or_missing_answer() {
        let v = decision_json(None);
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    }
}
