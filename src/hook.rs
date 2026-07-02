use crate::Config;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
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

/// Builds the spoken confirmation question for a PreToolUse hook invocation.
///
/// Only the `command` field of `tool_input` is special-cased (Bash's shell
/// command string) so it reads naturally when spoken. Every other tool falls
/// back to reading the raw `tool_input` JSON verbatim, which is serviceable
/// for today's Bash-only allowlist but should be tightened with per-tool
/// formatting before this is widened to other tools (e.g. Edit/Write/WebFetch)
/// whose JSON doesn't read naturally aloud.
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
        // Deliberately NOT single-character keys like "y"/"n": `match_choice`
        // (src/router.rs) does substring matching over the raw transcript,
        // so a one-letter key collides with common negative-response words
        // ("no wa*y*", "*y*et", "definitel*y*"). "deny" and "allow" are
        // long enough to avoid that class of accidental substring match.
        //
        // "deny" is listed FIRST — defense in depth. `match_choice` checks
        // choices in list order and returns on the first match, so if some
        // other unanticipated substring collision ever slips in, checking
        // the deny option first at least biases the outcome toward the
        // fail-safe (deny) side rather than the risky (allow) side.
        vec![
            ("deny".to_string(), "deny".to_string()),
            ("allow".to_string(), "allow".to_string()),
        ],
    )
}

/// PreToolUse `hookSpecificOutput` contract (confirmed against Claude Code
/// docs, see docs/plans/2026-06-30-voice-permission-hooks-implementation.md Task 0):
/// ```json
/// {"hookSpecificOutput": {"hookEventName": "PreToolUse",
///   "permissionDecision": "allow"|"deny"|"ask", "permissionDecisionReason": "..."}}
/// ```
/// Omitting stdout entirely (exit 0) falls through to Claude Code's
/// interactive prompt — that fallback lives in the CLI wiring, not here.
///
/// Maps a PTT answer to Claude Code's PreToolUse decision JSON.
/// `None` (timeout, or no answer) fails safe to deny.
pub fn decision_json(answer: Option<&str>) -> serde_json::Value {
    // "allow" is the key `pretooluse_question` actually produces and the
    // one the real caller (`run_pre_tool_use`) resolves through the router.
    // "y"/"yes" are kept as lenient fallbacks for other potential callers
    // (e.g. hand-rolled `callout ask` invocations) — not dead code, just not
    // reachable from the production PreToolUse path anymore.
    let allow = matches!(
        answer.map(str::to_lowercase).as_deref(),
        Some("allow") | Some("y") | Some("yes")
    );
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": if allow { "allow" } else { "deny" },
            "permissionDecisionReason": if allow { "approved via voice" } else { "denied via voice (or no response)" }
        }
    })
}

/// Default timeout (seconds) for the voice-approval `ask()` call made from
/// `run_pre_tool_use`, and the default for the standalone `callout ask` CLI
/// command.
///
/// IMPORTANT for whoever wires this into `.claude/settings.json` (Task 8):
/// Claude Code applies its own timeout to hook execution (defaulting to
/// 600s unless the hook's settings.json entry sets a `timeout` field), which
/// is a separate, independently-configured limit from this constant — don't
/// assume it covers you just because today's 600s default happens to exceed
/// `DEFAULT_ASK_TIMEOUT_SECS`. Since a PreToolUse hook invocation can
/// legitimately block here for up to `DEFAULT_ASK_TIMEOUT_SECS` waiting for
/// a PTT voice answer, the settings.json entry for this hook MUST still
/// explicitly set its own `timeout` field to at least this many seconds —
/// otherwise Claude Code will kill the `callout hook pre-tool-use` process
/// before the daemon ever gets to respond (whether because the default
/// changes upstream or this constant grows), and the tool call silently
/// falls through to Claude Code's default (non-voice) permission behavior.
pub const DEFAULT_ASK_TIMEOUT_SECS: u64 = 120;

/// Dispatches a `callout hook <cmd>` subcommand to its entry point.
pub fn run(cmd: crate::cli::HookCmd) -> Result<()> {
    match cmd {
        crate::cli::HookCmd::SessionStart => run_session_start(),
        crate::cli::HookCmd::PreToolUse => run_pre_tool_use(),
        crate::cli::HookCmd::Stop => run_stop(),
    }
}

fn read_stdin_json() -> Result<serde_json::Value> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("failed to read hook stdin")?;
    serde_json::from_str(&buf).context("failed to parse hook stdin as JSON")
}

fn daemon_base_url() -> String {
    format!(
        "http://127.0.0.1:{}",
        Config::load().unwrap_or_default().port
    )
}

fn agent_name_for(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Claude Code")
        .to_string()
}

/// $CLAUDE_SESSION_ID, or "manual" when running outside a Claude Code
/// session (e.g. a human invoking `callout notify`/`callout ask` directly).
fn cli_session_id() -> String {
    std::env::var("CLAUDE_SESSION_ID").unwrap_or_else(|_| "manual".into())
}

/// Common fields every hook stdin payload carries, plus the resolved daemon
/// base URL. Centralizes the "read stdin -> parse -> pull session_id/cwd"
/// boilerplate shared by all three `run_*` hook entry points.
struct HookContext {
    base: String,
    session_id: String,
    cwd: String,
}

impl HookContext {
    /// Reads and parses the hook JSON payload from stdin, returning both the
    /// common context and the raw payload (for callers like
    /// `run_pre_tool_use` that need additional fields, e.g. `tool_name`).
    fn from_stdin() -> Result<(Self, serde_json::Value)> {
        let payload = read_stdin_json()?;
        let ctx = Self {
            base: daemon_base_url(),
            session_id: payload["session_id"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
            cwd: payload["cwd"].as_str().unwrap_or(".").to_string(),
        };
        Ok((ctx, payload))
    }

    fn resolve_agent_id(&self) -> Result<String> {
        resolve_agent_id(
            &self.base,
            &Config::sessions_path(),
            &self.session_id,
            &agent_name_for(&self.cwd),
        )
    }
}

pub fn run_session_start() -> Result<()> {
    let (ctx, _payload) = HookContext::from_stdin()?;

    // Daemon unreachable at session start is non-fatal — later hook calls
    // will lazily register instead.
    if let Err(e) = ctx.resolve_agent_id() {
        tracing::warn!(error = %e, "callout daemon unreachable at SessionStart — will retry lazily");
    }
    Ok(())
}

pub fn run_stop() -> Result<()> {
    let (ctx, _payload) = HookContext::from_stdin()?;

    let agent_id = match ctx.resolve_agent_id() {
        Ok(id) => id,
        Err(e) => {
            // Fail-safe: daemon down, don't block Claude Code exiting the turn.
            tracing::warn!(error = %e, "callout daemon unreachable at Stop — skipping notification");
            return Ok(());
        }
    };
    let _ = notify(&ctx.base, &agent_id, "finished responding");
    Ok(())
}

pub fn run_pre_tool_use() -> Result<()> {
    let (ctx, payload) = HookContext::from_stdin()?;
    let tool_name = payload["tool_name"]
        .as_str()
        .unwrap_or("a tool")
        .to_string();
    let tool_input = payload
        .get("tool_input")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    let agent_id = match ctx.resolve_agent_id() {
        Ok(id) => id,
        Err(e) => {
            // Fail-safe: daemon unreachable -> print nothing, fall through to
            // Claude Code's normal interactive permission prompt.
            tracing::warn!(error = %e, "callout daemon unreachable at PreToolUse — falling through to normal permission prompt");
            return Ok(());
        }
    };

    let (question, choices) =
        pretooluse_question(&agent_name_for(&ctx.cwd), &tool_name, &tool_input);
    let result = ask(
        &ctx.base,
        &agent_id,
        &question,
        &choices,
        DEFAULT_ASK_TIMEOUT_SECS,
        Some("n"),
    );

    match result {
        Ok(r) => {
            println!("{}", decision_json(r.answer.as_deref()));
        }
        Err(e) => {
            tracing::warn!(error = %e, "ask call failed — falling through to normal permission prompt");
        }
    }
    Ok(())
}

pub fn run_notify(message: &str, agent_id: Option<&str>) -> Result<()> {
    let base = daemon_base_url();
    let agent_id = match agent_id {
        Some(id) => id.to_string(),
        None => resolve_agent_id(
            &base,
            &Config::sessions_path(),
            &cli_session_id(),
            "callout-cli",
        )?,
    };
    notify(&base, &agent_id, message)
}

pub fn run_ask(
    question: &str,
    choices: &[String],
    timeout: u64,
    default: Option<&str>,
    agent_id: Option<&str>,
) -> Result<()> {
    let base = daemon_base_url();
    let agent_id = match agent_id {
        Some(id) => id.to_string(),
        None => resolve_agent_id(
            &base,
            &Config::sessions_path(),
            &cli_session_id(),
            "callout-cli",
        )?,
    };
    let parsed_choices: Vec<(String, String)> = choices
        .iter()
        .filter_map(|c| c.split_once(':'))
        .map(|(k, l)| (k.to_string(), l.to_string()))
        .collect();
    let result = ask(
        &base,
        &agent_id,
        question,
        &parsed_choices,
        timeout,
        default,
    )?;
    match result.answer {
        Some(a) => println!("{a}"),
        None => println!(
            "(no answer{})",
            if result.timed_out { ", timed out" } else { "" }
        ),
    }
    Ok(())
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
                ("deny".to_string(), "deny".to_string()),
                ("allow".to_string(), "allow".to_string())
            ]
        );
    }

    /// `match_choice` (src/router.rs) checks choices in list order and
    /// returns on the first substring match. `pretooluse_question` puts
    /// "deny" before "allow" deliberately, as defense in depth for this
    /// fail-safe-biased decision: if an unanticipated substring collision
    /// ever slips in, checking deny first biases the outcome toward the
    /// safe (deny) side rather than the risky (allow) side. This test pins
    /// that ordering so a future refactor can't silently flip it.
    #[test]
    fn pretooluse_question_lists_deny_before_allow() {
        let (_q, choices) =
            pretooluse_question("Test Agent", "Bash", &json!({"command": "rm -rf dist"}));
        assert_eq!(
            choices[0].0, "deny",
            "deny must be checked first by match_choice"
        );
        assert_eq!(choices[1].0, "allow");
    }

    #[test]
    fn decision_json_approve_on_allow() {
        let v = decision_json(Some("allow"));
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
    }

    #[test]
    fn decision_json_approve_on_yes_lenient_fallback() {
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
