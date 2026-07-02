# Voice-Driven Permission Hooks — Design

## Motivation

Today, every Callout integration depends on an agent voluntarily `curl`-ing `/notify` or `/ask`. That's unreliable — nothing forces it, and agents forget mid-task. It also means Callout is purely reactive: the only way the user can speak to an agent is to answer an `/ask` the agent chose to make. There's no deterministic, harness-enforced moment where the agent is guaranteed to check in.

Claude Code's hook system fixes this: hooks are invoked by the harness itself at well-defined event points, regardless of what the agent decides. Wiring Callout into hooks — instead of relying on agent discretion — converts "the agent might call Callout" into "the harness always calls Callout."

Scope for this pass: reliable hook-driven `/notify` and `/ask` calls (including voice-driven tool permission approval), plus a thin `callout` CLI wrapper. Explicitly out of scope: true async push into a running agent (tmux/pty-based proactive interrupt) — see "Future: session management" below.

## Hook → Callout mapping

- **`SessionStart`** → `callout hook session-start` reads the hook's stdin JSON (`session_id`, `cwd`), derives an agent name from the repo/dir, `POST /agents/register`, and caches `session_id → agent_id` in `~/.callout/sessions.json`.
- **`PreToolUse`** (scoped to a tool allowlist — e.g. `Bash` — not every tool call) → `callout hook pre-tool-use` looks up `agent_id`, `POST /ask` with a yes/no question built from `tool_name`/`tool_input`, blocks on the user's PTT answer, and translates the result into Claude Code's hook decision JSON. This replaces keyboard y/n permission prompts with voice, for whichever tools are scoped in.
- **`Stop`** → `callout hook stop` pulls the last assistant message from the transcript path the hook provides, `POST /notify` with a short gist, so the user always hears when a turn ends.
- **`Notification`** (optional, lower priority) → forwards Claude Code's idle/permission-needed text to `/notify` as an FYI for cases `PreToolUse` doesn't cover.

## CLI subcommands + registry

All `callout hook *` subcommands are thin: parse hook JSON from stdin, look up or lazily create an `agent_id`, call the existing HTTP API, translate the response back into whatever Claude Code's hook contract expects.

- Registry: `~/.callout/sessions.json`, a map `session_id → {agent_id, registered_at}`. Every `callout hook *` subcommand reads `session_id` from stdin JSON and looks it up; if missing, or the daemon 404s on it (e.g. after a restart), it lazily re-registers. No subcommand hard-fails just because `session-start` didn't run first or the daemon bounced.
- `callout hook session-start` — registers explicitly, seeds the registry entry.
- `callout hook pre-tool-use` — reads `tool_name`/`tool_input`, builds a yes/no question, `POST /ask`, maps the answer to Claude Code's `{"decision": "approve"|"block", "reason": ...}` JSON on stdout.
- `callout hook stop` — reads `transcript_path`, grabs the last assistant message, `POST /notify`.
- Plain `callout notify <msg>` / `callout ask <q> [--choice k:label]...` sit underneath these as general-purpose, scriptable commands — the `hook` subcommands just supply the message/question programmatically instead of a human typing it. Same registration/lookup code path either way.

Wiring: `.claude/settings.json` hooks config points each event at `callout hook <name>` — Claude Code pipes the JSON payload via stdin automatically, no `jq` needed.

The registry format is intentionally the seed of a future session-manager (see below) — keep it extensible rather than single-purpose.

## Data flow (PreToolUse example)

1. Claude Code is about to run `Bash` → invokes the hook with `{session_id, tool_name, tool_input, transcript_path}` on stdin.
2. `callout hook pre-tool-use` resolves `agent_id` (registering if needed), builds `"<agent> wants to run: <command>. Allow?"` with `y`/`n` choices and `default: "n"`.
3. `POST /ask` — daemon speaks it via TTS, blocks.
4. User holds PTT, replies.
5. Response maps to `{"decision": "approve"}` or `{"decision": "block", "reason": "denied via voice"}` on stdout.
6. Claude Code applies the decision.

## Error handling — fail-safe, not fail-open

- Daemon unreachable → hook exits with **no decision output** at all, so Claude Code silently falls back to its normal interactive permission prompt. Never auto-approve just because Callout is down.
- `/ask` times out (no PTT reply) → uses `default: "n"` (deny), not approve — already supported by the existing API's `default` field.
- Stale `agent_id` after a daemon restart → self-heals by re-registering (see registry above).

## Testing

- Unit-test the pure mapping functions (`hook stdin JSON → AskRequest`, `AskResponseBody → PreToolUse decision JSON`) — no daemon needed, fits the existing `tests/api.rs` style.
- Scope `PreToolUse` to just `Bash` first in this repo's own `.claude/settings.json` and dogfood it directly.

## Future: session management (not in this pass)

Long-term, Callout should be able to launch and manage Claude Code sessions itself (spawn, list, switch, kill), not just receive calls from sessions that already exist. True proactive push (user speaks without a pending `/ask`, message reaches a running agent) was considered via `tmux send-keys`, but that only works for tmux users. The better direction is a `callout run -- claude` wrapper that spawns the agent under a pty Callout owns directly — this works regardless of terminal/multiplexer, gives Callout a session handle without any hook needed for registration, and is the natural foundation for a future `callout sessions` command (list/attach/start/kill).
