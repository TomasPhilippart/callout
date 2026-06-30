# Voice-Driven Permission Hooks Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make Callout's `/notify` and `/ask` calls harness-enforced via Claude Code hooks (instead of agent-discretionary `curl`), including voice-driven yes/no tool-permission approval.

**Architecture:** New `src/hook.rs` module + 3 CLI subcommands (`callout hook session-start|pre-tool-use|stop`) that read Claude Code's hook JSON from stdin, resolve a `session_id -> agent_id` mapping cached at `~/.callout/sessions.json` (self-healing against daemon restarts via `/status`), and call the existing `/agents/register`, `/notify`, `/ask` HTTP endpoints with a new blocking `ureq` client. Also adds plain `callout notify`/`callout ask` commands for manual/scripted use, sharing the same resolution code path.

**Tech Stack:** Rust, clap (existing), serde_json (existing), `ureq` (new — blocking HTTP client, appropriate since these CLI subcommands run synchronously outside the tokio runtime, same as `voices`/`model`/`logs`).

**Design doc:** `docs/plans/2026-06-30-voice-permission-hooks-design.md`

---

### Task 0: Confirm the PreToolUse hook JSON contract

Claude Code's exact hook stdin/stdout schema can change between versions — do not trust this plan's guesses below without checking.

**Step 1:** Ask the `claude-code-guide` agent (or check current docs): "What is the exact JSON Claude Code sends on stdin to a PreToolUse hook, and what JSON/exit-code contract does the hook use to allow or deny the tool call (not just fall through to the interactive prompt)?" Also confirm the `SessionStart` and `Stop` hook stdin schemas.

**Step 2:** Record the confirmed field names as a doc comment at the top of `src/hook.rs` (created in Task 2) before writing `decision_json` in Task 6. If the schema below turns out to be wrong, fix it there — don't guess silently.

Best-known schema going in (verify before relying on it):
- All hooks receive on stdin: `{"session_id": "...", "transcript_path": "...", "cwd": "...", "hook_event_name": "..."}`.
- `PreToolUse` adds: `"tool_name": "Bash", "tool_input": {"command": "...", ...}`.
- `PreToolUse` stdout JSON to control the decision: `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "allow"|"deny"|"ask", "permissionDecisionReason": "..."}}`. Omitting stdout (exit 0) falls through to the normal interactive prompt — that's the fail-safe default when Callout can't be reached.

---

### Task 1: Add `ureq` dependency

**Files:**
- Modify: `Cargo.toml`

**Step 1:** Add to `[dependencies]`:

```toml
ureq         = { version = "2", features = ["json"] }
```

**Step 2:** Verify it fetches and the crate still builds.

```bash
cargo build 2>&1 | tail -10
```
Expected: no errors.

**Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: add ureq for synchronous CLI HTTP calls to the local daemon"
```

---

### Task 2: Session registry (`~/.callout/sessions.json`) — load/save

**Files:**
- Create: `src/hook.rs`
- Modify: `src/lib.rs` (add `pub mod hook;`)
- Modify: `src/config.rs` (add `sessions_path()` helper)

**Step 1: Add `Config::sessions_path()`**

In `src/config.rs`, after `pub fn logs_dir()`:

```rust
pub fn sessions_path() -> PathBuf {
    Self::dir().join("sessions.json")
}
```

**Step 2: Write the failing tests**

Create `src/hook.rs`:

```rust
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
```

**Step 3: Wire the module**

In `src/lib.rs`, add alphabetically among the `pub mod` lines:

```rust
pub mod hook;
```

**Step 4: Run tests**

```bash
cargo test --lib hook:: 2>&1 | tail -20
```
Expected: 3 passed.

**Step 5: Commit**

```bash
git add src/hook.rs src/lib.rs src/config.rs
git commit -m "feat: add session registry load/save for hook agent_id caching"
```

---

### Task 3: HTTP client functions against the daemon

**Files:**
- Modify: `src/hook.rs`
- Create: `tests/hook_client.rs`

These wrap `ureq` calls to the existing `/agents/register`, `/notify`, `/ask`, `/status` endpoints. They need a real bound TCP listener to test against (the existing `tests/api.rs` uses `tower::ServiceExt::oneshot`, which doesn't open a real socket — `ureq` needs one).

**Step 1: Add a test harness that boots the real server on an ephemeral port**

Create `tests/hook_client.rs`:

```rust
use callout::{
    agents::AgentRegistry, api, glossary::Glossary, recorder::Recorder, router::AskRouter,
    AppState, Config,
};
use std::sync::{atomic::AtomicBool, Arc};
use tokio::sync::{mpsc, Mutex, Notify, RwLock};

fn test_state() -> AppState {
    let (tts_tx, _tts_rx) = mpsc::channel(8);
    AppState {
        agents: Arc::new(RwLock::new(AgentRegistry::new())),
        router: Arc::new(Mutex::new(AskRouter::new())),
        config: Arc::new(Config::default()),
        glossary: Arc::new(Glossary::default()),
        tts_tx,
        ptt_recorder: Arc::new(Mutex::new(Recorder::default())),
        transcriber: None,
        recording: Arc::new(AtomicBool::new(false)),
        tts_speaking: Arc::new(AtomicBool::new(false)),
        just_processed: Arc::new(AtomicBool::new(false)),
        tts_kill: Arc::new(Notify::new()),
        active_agent: Arc::new(std::sync::Mutex::new(None)),
    }
}

/// Boots a real callout API server on an ephemeral localhost port.
/// Returns the base URL (e.g. "http://127.0.0.1:54321").
async fn spawn_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = api::build_app(test_state());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn register_agent_returns_id() {
    let base = spawn_server().await;
    let id = tokio::task::spawn_blocking(move || {
        callout::hook::register_agent(&base, "Test Agent")
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(id.len(), 6);
}

#[tokio::test]
async fn notify_succeeds() {
    let base = spawn_server().await;
    let id = tokio::task::spawn_blocking({
        let base = base.clone();
        move || callout::hook::register_agent(&base, "Test Agent")
    })
    .await
    .unwrap()
    .unwrap();

    tokio::task::spawn_blocking(move || callout::hook::notify(&base, &id, "hello"))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn status_lists_registered_agent() {
    let base = spawn_server().await;
    let id = tokio::task::spawn_blocking({
        let base = base.clone();
        move || callout::hook::register_agent(&base, "Test Agent")
    })
    .await
    .unwrap()
    .unwrap();

    let ids = tokio::task::spawn_blocking(move || callout::hook::status_agent_ids(&base))
        .await
        .unwrap()
        .unwrap();
    assert!(ids.contains(&id));
}
```

**Step 2: Run — expect compile failure (functions don't exist yet)**

```bash
cargo test --test hook_client 2>&1 | tail -20
```
Expected: `error[E0425]` / `no function named 'register_agent'` etc.

**Step 3: Implement the client functions**

Append to `src/hook.rs`:

```rust
use serde::de::DeserializeOwned;

fn http_error(context: &str, e: ureq::Error) -> anyhow::Error {
    anyhow::anyhow!("{context}: {e}")
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
```

Note: `fn http_error` takes ownership of `DeserializeOwned` import only if needed — drop the unused import if `cargo build` flags it.

**Step 4: Run tests — expect pass**

```bash
cargo test --test hook_client 2>&1 | tail -20
```
Expected: 3 passed.

**Step 5: Commit**

```bash
git add src/hook.rs tests/hook_client.rs
git commit -m "feat: add blocking HTTP client functions for register/notify/ask/status"
```

---

### Task 4: `resolve_agent_id` — self-healing session → agent_id lookup

**Files:**
- Modify: `src/hook.rs`
- Modify: `tests/hook_client.rs`

**Step 1: Write the failing test**

Add to `tests/hook_client.rs`:

```rust
#[tokio::test]
async fn resolve_agent_id_registers_once_and_reuses() {
    let base = spawn_server().await;
    let dir = std::env::temp_dir().join(format!("callout-test-resolve-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("sessions.json");

    let (base1, path1) = (base.clone(), path.clone());
    let id1 = tokio::task::spawn_blocking(move || {
        callout::hook::resolve_agent_id(&base1, &path1, "sess-a", "Test Agent")
    })
    .await
    .unwrap()
    .unwrap();

    let (base2, path2) = (base.clone(), path.clone());
    let id2 = tokio::task::spawn_blocking(move || {
        callout::hook::resolve_agent_id(&base2, &path2, "sess-a", "Test Agent")
    })
    .await
    .unwrap()
    .unwrap();

    assert_eq!(id1, id2, "second call must reuse the cached agent_id");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn resolve_agent_id_reregisters_after_daemon_restart() {
    let base = spawn_server().await;
    let dir = std::env::temp_dir().join(format!("callout-test-stale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("sessions.json");

    // Pretend a previous daemon instance registered this session.
    let mut registry = callout::hook::Registry::new();
    registry.insert(
        "sess-b".into(),
        callout::hook::SessionEntry {
            agent_id: "stale1".into(),
        },
    );
    callout::hook::save_registry(&path, &registry).unwrap();

    let (base2, path2) = (base.clone(), path.clone());
    let id = tokio::task::spawn_blocking(move || {
        callout::hook::resolve_agent_id(&base2, &path2, "sess-b", "Test Agent")
    })
    .await
    .unwrap()
    .unwrap();

    assert_ne!(id, "stale1", "must re-register when the daemon doesn't know the cached id");
    let _ = std::fs::remove_dir_all(&dir);
}
```

**Step 2: Run — expect failure**

```bash
cargo test --test hook_client resolve_agent_id 2>&1 | tail -20
```
Expected: compile error, `resolve_agent_id` not found.

**Step 3: Implement**

Append to `src/hook.rs`:

```rust
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
        tracing::info!(session_id, "cached agent_id is stale — re-registering");
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
```

**Step 4: Run tests — expect pass**

```bash
cargo test --test hook_client 2>&1 | tail -20
```
Expected: all pass.

**Step 5: Commit**

```bash
git add src/hook.rs tests/hook_client.rs
git commit -m "feat: add resolve_agent_id with daemon-restart self-healing"
```

---

### Task 5: Pure mapping functions (PreToolUse question + decision JSON)

**Files:**
- Modify: `src/hook.rs`

No I/O — fast unit tests. **Use the schema confirmed in Task 0**, not necessarily the placeholder below.

**Step 1: Write the failing tests**

Add to the bottom of `src/hook.rs` (new `mod tests` additions, alongside the existing ones from Task 2):

```rust
#[test]
fn pretooluse_question_includes_command_for_bash() {
    let (q, choices) = pretooluse_question("Test Agent", "Bash", &json!({"command": "rm -rf dist"}));
    assert!(q.contains("Test Agent"));
    assert!(q.contains("rm -rf dist"));
    assert_eq!(choices, vec![("y".to_string(), "yes".to_string()), ("n".to_string(), "no".to_string())]);
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
```

Add `use serde_json::json;` to the test module's imports.

**Step 2: Run — expect failure**

```bash
cargo test --lib hook:: 2>&1 | tail -20
```
Expected: compile errors, functions not found.

**Step 3: Implement**

Append to `src/hook.rs` (outside the test module):

```rust
pub fn pretooluse_question(agent_name: &str, tool_name: &str, tool_input: &serde_json::Value) -> (String, Vec<(String, String)>) {
    let detail = tool_input
        .get("command")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| tool_input.to_string());
    let question = format!("{agent_name} wants to run {tool_name}: {detail}. Allow?");
    (
        question,
        vec![("y".to_string(), "yes".to_string()), ("n".to_string(), "no".to_string())],
    )
}

/// Maps a PTT answer to Claude Code's PreToolUse decision JSON.
/// `None` (timeout, or no answer) fails safe to deny.
pub fn decision_json(answer: Option<&str>) -> serde_json::Value {
    let allow = matches!(answer.map(str::to_lowercase).as_deref(), Some("y") | Some("yes"));
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": if allow { "allow" } else { "deny" },
            "permissionDecisionReason": if allow { "approved via voice" } else { "denied via voice (or no response)" }
        }
    })
}
```

**Step 4: Run tests — expect pass**

```bash
cargo test --lib hook:: 2>&1 | tail -20
```
Expected: all pass.

**Step 5: Commit**

```bash
git add src/hook.rs
git commit -m "feat: add pure PreToolUse question/decision mapping functions"
```

---

### Task 6: CLI subcommands — `callout hook ...`, `callout notify`, `callout ask`

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/hook.rs` (the `run_*` entry points)

**Step 1: Extend `Command` enum**

In `src/cli.rs`, add to the `Command` enum (after `Logs`):

```rust
/// Send a one-off spoken notification
Notify {
    message: String,
    /// Explicit agent_id to attach to (default: reuses a session keyed by
    /// $CLAUDE_SESSION_ID, or "manual" if unset)
    #[arg(long)]
    agent_id: Option<String>,
},
/// Ask a yes/no or multiple-choice question and wait for a PTT answer
Ask {
    question: String,
    /// Choice in "key:label" form, repeatable
    #[arg(long = "choice")]
    choices: Vec<String>,
    #[arg(long, default_value = "120")]
    timeout: u64,
    #[arg(long)]
    default: Option<String>,
    #[arg(long)]
    agent_id: Option<String>,
},
/// Claude Code hook entry points (read hook JSON from stdin)
Hook {
    #[command(subcommand)]
    cmd: HookCmd,
},
```

Add the new subcommand enum at the bottom of the file:

```rust
#[derive(Subcommand)]
pub enum HookCmd {
    /// SessionStart hook: registers this session as an agent
    SessionStart,
    /// PreToolUse hook: voice-approve/deny the pending tool call
    PreToolUse,
    /// Stop hook: notify that the agent finished a turn
    Stop,
}
```

**Step 2: Implement entry points in `src/hook.rs`**

Append:

```rust
use std::io::Read;

fn read_stdin_json() -> Result<serde_json::Value> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("failed to read hook stdin")?;
    serde_json::from_str(&buf).context("failed to parse hook stdin as JSON")
}

fn daemon_base_url() -> String {
    let port = Config::load().map(|c| c.port).unwrap_or(7878);
    format!("http://127.0.0.1:{port}")
}

fn agent_name_for(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Claude Code")
        .to_string()
}

pub fn run_session_start() -> Result<()> {
    let payload = read_stdin_json()?;
    let session_id = payload["session_id"].as_str().unwrap_or("unknown").to_string();
    let cwd = payload["cwd"].as_str().unwrap_or(".").to_string();
    let base = daemon_base_url();

    // Daemon unreachable at session start is non-fatal — later hook calls
    // will lazily register instead.
    if let Err(e) = resolve_agent_id(&base, &Config::sessions_path(), &session_id, &agent_name_for(&cwd)) {
        tracing::warn!(error = %e, "callout daemon unreachable at SessionStart — will retry lazily");
    }
    Ok(())
}

pub fn run_stop() -> Result<()> {
    let payload = read_stdin_json()?;
    let session_id = payload["session_id"].as_str().unwrap_or("unknown").to_string();
    let cwd = payload["cwd"].as_str().unwrap_or(".").to_string();
    let base = daemon_base_url();

    let Ok(agent_id) = resolve_agent_id(&base, &Config::sessions_path(), &session_id, &agent_name_for(&cwd)) else {
        return Ok(()); // fail-safe: daemon down, don't block Claude Code exiting the turn
    };
    let _ = notify(&base, &agent_id, "finished responding");
    Ok(())
}

pub fn run_pre_tool_use() -> Result<()> {
    let payload = read_stdin_json()?;
    let session_id = payload["session_id"].as_str().unwrap_or("unknown").to_string();
    let cwd = payload["cwd"].as_str().unwrap_or(".").to_string();
    let tool_name = payload["tool_name"].as_str().unwrap_or("a tool").to_string();
    let tool_input = payload.get("tool_input").cloned().unwrap_or(serde_json::json!({}));
    let base = daemon_base_url();

    let Ok(agent_id) = resolve_agent_id(&base, &Config::sessions_path(), &session_id, &agent_name_for(&cwd)) else {
        // Fail-safe: daemon unreachable -> print nothing, fall through to
        // Claude Code's normal interactive permission prompt.
        return Ok(());
    };

    let (question, choices) = pretooluse_question(&agent_name_for(&cwd), &tool_name, &tool_input);
    let result = ask(&base, &agent_id, &question, &choices, 120, Some("n"));

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
        None => {
            let session_id = std::env::var("CLAUDE_SESSION_ID").unwrap_or_else(|_| "manual".into());
            resolve_agent_id(&base, &Config::sessions_path(), &session_id, "callout-cli")?
        }
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
        None => {
            let session_id = std::env::var("CLAUDE_SESSION_ID").unwrap_or_else(|_| "manual".into());
            resolve_agent_id(&base, &Config::sessions_path(), &session_id, "callout-cli")?
        }
    };
    let parsed_choices: Vec<(String, String)> = choices
        .iter()
        .filter_map(|c| c.split_once(':'))
        .map(|(k, l)| (k.to_string(), l.to_string()))
        .collect();
    let result = ask(&base, &agent_id, question, &parsed_choices, timeout, default)?;
    match result.answer {
        Some(a) => println!("{a}"),
        None => println!("(no answer{})", if result.timed_out { ", timed out" } else { "" }),
    }
    Ok(())
}
```

Add `use std::path::Path;` to the top imports if not already present from Task 2.

**Step 3: Wire dispatch in `src/main.rs`**

Add match arms:

```rust
Some(Command::Notify { message, agent_id }) => {
    callout::hook::run_notify(&message, agent_id.as_deref())
}
Some(Command::Ask { question, choices, timeout, default, agent_id }) => {
    callout::hook::run_ask(&question, &choices, timeout, default.as_deref(), agent_id.as_deref())
}
Some(Command::Hook { cmd }) => match cmd {
    callout::cli::HookCmd::SessionStart => callout::hook::run_session_start(),
    callout::cli::HookCmd::PreToolUse => callout::hook::run_pre_tool_use(),
    callout::cli::HookCmd::Stop => callout::hook::run_stop(),
},
```

**Step 4: Verify it compiles**

```bash
cargo check 2>&1 | grep "error\[" 
```
Expected: no output.

**Step 5: Run full test suite**

```bash
cargo test 2>&1 | tail -30
```
Expected: all pass.

**Step 6: Commit**

```bash
git add src/cli.rs src/main.rs src/hook.rs
git commit -m "feat: add 'callout hook' subcommands and plain notify/ask CLI commands"
```

---

### Task 7: `make check` and manual smoke test

**Step 1: Run the full check suite**

```bash
make check
```
Expected: fmt clean, clippy clean (`-D warnings`), all tests pass. Fix any clippy complaints (likely: unused imports, needless clones) before proceeding.

**Step 2: Manual smoke test against a running daemon**

With `callout serve` running locally (or just `cargo run -- serve` in another terminal):

```bash
echo '{"session_id":"manual-test","cwd":"'"$(pwd)"'","hook_event_name":"SessionStart"}' | cargo run -- hook session-start
cat ~/.callout/sessions.json   # should show an entry for "manual-test"

echo '{"session_id":"manual-test","cwd":"'"$(pwd)"'","hook_event_name":"Stop"}' | cargo run -- hook stop
# Expect: TTS speaks "<dirname>: finished responding"

cargo run -- notify "manual smoke test" --agent-id "$(cat ~/.callout/sessions.json | python3 -c 'import json,sys; print(json.load(sys.stdin)["manual-test"]["agent_id"])')"
# Expect: TTS speaks the message
```

**Step 3: Commit if clippy required fixes**

```bash
git add -p
git commit -m "chore: fix clippy warnings in hook module"
```

---

### Task 8: Wire hooks into this repo's `.claude/settings.json` and dogfood

**Files:**
- Create: `.claude/settings.json` (if it doesn't already exist — check first, this may need merging with existing hook config)

**Step 1: Check for an existing settings file**

```bash
cat .claude/settings.json 2>/dev/null || echo "none"
```

**Step 2: Add (or merge in) hook config**

```json
{
  "hooks": {
    "SessionStart": [
      { "hooks": [{ "type": "command", "command": "callout hook session-start" }] }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "callout hook stop" }] }
    ],
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [{ "type": "command", "command": "callout hook pre-tool-use" }]
      }
    ]
  }
}
```

Note: `command` must resolve on `$PATH` — confirm `callout` is installed (`cargo install --path .` or symlinked) before relying on this in a real session, otherwise use the absolute path to the built binary.

**Step 3: Restart Claude Code in this repo (new session) and verify**

- On session start, `~/.callout/sessions.json` should gain an entry.
- Trigger a `Bash` tool call — you should hear a spoken yes/no prompt instead of (or alongside) the normal permission UI, and your PTT answer should allow/deny it.
- End the turn — you should hear "finished responding".

**Step 4: Commit**

```bash
git add .claude/settings.json
git commit -m "feat: wire voice permission hooks into this repo's Claude Code settings"
```

---

### Task 9: Final review

**Step 1:** Re-read `docs/plans/2026-06-30-voice-permission-hooks-design.md` and confirm the implementation matches (especially the fail-safe error handling section).

**Step 2:** `superpowers:requesting-code-review` before merging.
