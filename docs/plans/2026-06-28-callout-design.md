# Callout — Design Document
_2026-06-28_

## What It Is

Callout is a lightweight background daemon for macOS/Linux that gives developers an ambient voice interface to their AI agents. It runs silently, speaks up when an agent needs a decision, listens for your response, and routes the answer back — no window switching, no terminal babysitting.

---

## Architecture

Single Rust binary. Tokio async runtime. Tasks communicate over internal channels.

```
┌─────────────────────────────────────────────────────────┐
│                    Callout (single binary)               │
│                                                         │
│  ┌─────────────┐  ┌──────────────┐  ┌───────────────┐  │
│  │  HTTP API   │  │ Audio        │  │  Menu Bar     │  │
│  │  axum       │  │ Pipeline     │  │  tray-icon    │  │
│  │  :7878      │  │ cpal+whisper │  │               │  │
│  └──────┬──────┘  └──────┬───────┘  └───────┬───────┘  │
│         └────────────────┴──────────────────┘           │
│                     State Manager                        │
│              (tokio channels + shared state)             │
└─────────────────────────────────────────────────────────┘
        ↑                          ↑
  any agent (curl,          global hotkey
  shell script, SDK)        (push-to-talk)
```

---

## HTTP API

Binds to `localhost:7878` (configurable). All payloads are JSON.

### Agent registration

**`POST /agents/register`**
```json
{ "name": "auth-refactor", "description": "Refactoring the auth module", "context_terms": ["OAuth", "Keycloak", "JWT"] }
```
```json
{ "agent_id": "a1b2c3" }
```

**`DELETE /agents/{agent_id}`** — deregister on shutdown. Agents not seen for 5 minutes are marked stale automatically (last-seen updated on every API call).

Registration is optional — agents that only use `/notify` without registering appear as `"unknown"`.

---

### `POST /notify`

Fire-and-forget. Callout speaks the message and logs it.

```json
{ "agent_id": "a1b2c3", "message": "Build finished, 3 tests failed" }
```

Returns `200` immediately.

---

### `POST /ask`

Blocking. Callout speaks the question, opens a listen window, transcribes the response, and returns it. Agent code just awaits the HTTP response.

```json
{
  "agent_id": "a1b2c3",
  "question": "Should I delete the generated files?",
  "choices": [
    { "key": "A", "label": "Yes, delete them" },
    { "key": "B", "label": "No, keep them" },
    { "key": "C", "label": "Skip for now" }
  ],
  "multi_select": false,
  "timeout_seconds": 120,
  "default": "C"
}
```

- `choices` — optional. When present, Callout reads the options aloud after the question. Omit for free-form voice answers.
- `multi_select` — default `false`. When `true`, user can say "A and C"; returns multiple keys.
- `timeout_seconds` + `default` — if no response within the timeout, uses the default and continues. Without a default, returns `timed_out: true` and `answer: null`.

**Response:**
```json
{
  "answer": "B",
  "answers": ["B"],
  "raw": "no keep them",
  "timed_out": false
}
```

`answer` — matched key (or raw transcript if no choices). `answers` — always an array (one element for single-select, multiple for multi-select). `raw` — what Whisper heard.

---

### `GET /status`

```json
{
  "agents": [
    { "id": "a1b2c3", "name": "auth-refactor", "state": "waiting", "last_seen": "5s ago" },
    { "id": "d4e5f6", "name": "test-runner",   "state": "idle",    "last_seen": "2m ago" }
  ]
}
```

---

## Audio Pipeline

Four tasks on Tokio channels:

```
Hotkey → Recorder (cpal) → Transcriber (whisper-rs) → Router
                                                          ↓
Speaker (say / piper) ←────────────────── HTTP /ask or /notify
```

- **Hotkey** (`global-hotkey` crate) — default `Option` on macOS, `Alt` on Linux. Hold = record, release = transcribe. Configurable.
- **Recorder** (`cpal`) — opens default input device at 16kHz mono f32 (Whisper's native format). Accumulates samples in a `Vec<f32>` while hotkey is held.
- **Transcriber** (`whisper-rs`) — receives audio buffer, runs through `whisper.cpp`. Model loaded once at startup from `~/.callout/models/ggml-base.bin`. Returns transcript string.
- **Speaker** — macOS: `std::process::Command::new("say")`. Linux: spawns `piper` piped to `aplay`. Holds a "speaking" lock so Recorder won't start while TTS is active (prevents echo).

**Interaction flows:**

_Agent asks a question:_
1. `POST /ask` arrives → Router registers a `oneshot::Sender`, suspends
2. Speaker speaks: _"auth-refactor asks: Should I delete these files? A: Yes. B: No."_
3. User presses hotkey → Recorder captures → Transcriber returns transcript
4. Router fuzzy-matches transcript against choices → resolves the sender → HTTP response returned

_User asks for status:_
1. User presses hotkey → Recorder captures → Transcriber returns transcript
2. Router detects no pending `/ask` → treats as a status query
3. Speaker responds: _"Two agents running. auth-refactor is waiting for your answer. test-runner is idle."_

---

## Glossary

`~/.callout/glossary.toml` — two mechanisms:

```toml
# Fed to Whisper as initial prompt on every transcription
# Biases Whisper toward these spellings over phonetically similar alternatives
terms = ["Claude", "Anthropic", "tokio", "axum", "kubectl", "Callout"]

# Hard find-and-replace applied after transcription
[corrections]
"Cloud" = "Claude"
"cloud" = "Claude"
```

Agents contribute additional terms at registration via `context_terms` — merged into the Whisper prompt for the duration of that agent's session.

---

## Menu Bar

`tray-icon` crate. Four icon states:

| State     | When                                     |
|-----------|------------------------------------------|
| Idle      | No agents registered                     |
| Active    | Agents running, nothing pending          |
| Waiting   | An agent is blocked on `/ask`            |
| Listening | Hotkey held, recording in progress       |

Click → dropdown:

```
● auth-refactor    [waiting]
● test-runner      [active]
──────────────────────────────
  auth-refactor asks: "Delete files?"
  test-runner: Build finished
  test-runner: Starting integration tests
──────────────────────────────
  Quit
```

On Linux: `tray-icon` falls back to `libappindicator` (GTK) or `ksni` (KDE).

---

## Configuration

`~/.callout/config.toml`:

```toml
port = 7878
model = "base"   # tiny | base | small | medium

[hotkey]
key = "Alt"      # Option on macOS

[tts]
# macOS: uses `say` automatically
# Linux: path to piper binary + voice model
piper_bin = "/usr/local/bin/piper"
piper_voice = "~/.callout/voices/en_US-amy-medium.onnx"
```

`~/.callout/` layout:

```
~/.callout/
├── config.toml
├── glossary.toml
├── models/          # Whisper GGML models
│   └── ggml-base.bin
├── voices/          # Piper voice models (Linux)
└── logs/
    └── callout.log
```

---

## Project Structure

```
callout/
├── Cargo.toml
└── src/
    ├── main.rs            # spawns all tasks, wires channels
    ├── config.rs          # ~/.callout/config.toml + ~/.callout/glossary.toml
    ├── api/
    │   ├── mod.rs         # axum router
    │   ├── notify.rs
    │   ├── ask.rs
    │   ├── agents.rs
    │   └── status.rs
    ├── audio/
    │   ├── recorder.rs    # cpal capture
    │   ├── speaker.rs     # say / piper
    │   └── transcriber.rs # whisper-rs
    ├── hotkey.rs          # global-hotkey task
    ├── tray.rs            # menu bar icon + dropdown
    ├── agents.rs          # agent registry + last-seen tracking
    ├── glossary.rs        # term loading + whisper prompt builder + corrections
    └── router.rs          # oneshot channel map: agent_id → pending /ask sender
```

Two binaries (single workspace):
- `callout` — daemon + menu bar. Also acts as CLI client (`callout notify "..."`, `callout status`) when the daemon is already running.

---

## Crate Dependencies

| Crate | Purpose |
|---|---|
| `tokio` | async runtime |
| `axum` | HTTP server |
| `whisper-rs` | whisper.cpp bindings (STT) |
| `cpal` | cross-platform audio I/O |
| `tray-icon` | menu bar icon |
| `global-hotkey` | push-to-talk hotkey |
| `serde` / `serde_json` | JSON |
| `toml` | config parsing |
| `strsim` | fuzzy string matching for choice selection |

---

## What's Out of Scope for v1

- MCP adapter
- Wake word (v2)
- Glossary learning / auto-correction from voice feedback (v2)
- Menu bar clickable choices for `/ask` (v2)
- Urgency levels for TTS tone (v2)
- Multi-language STT (v2)
