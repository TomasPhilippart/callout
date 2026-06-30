use std::sync::{atomic::Ordering, Arc};

use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};

use crate::{recorder::Recorder, transcriber::Transcriber, AppState};

/// 16 kHz mono: 0.1 s minimum to avoid transcribing accidental key taps.
const MIN_SAMPLES: usize = 1_600;

pub fn spawn(state: AppState, transcriber: Arc<Transcriber>) {
    std::thread::Builder::new()
        .name("callout-ptt".into())
        .spawn(move || run_loop(state, transcriber))
        .expect("failed to spawn PTT thread");
}

fn run_loop(state: AppState, transcriber: Arc<Transcriber>) {
    let ptt_str = state.config.hotkey.ptt.clone();

    let hotkey = match parse_hotkey(&ptt_str) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(key = %ptt_str, error = %e, "invalid PTT hotkey");
            return;
        }
    };

    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "failed to init hotkey manager");
            return;
        }
    };

    if let Err(e) = manager.register(hotkey) {
        tracing::error!(error = %e, "failed to register PTT hotkey");
        return;
    }

    tracing::info!(key = %ptt_str, "PTT ready — hold to speak, release to send");

    let receiver = GlobalHotKeyEvent::receiver();
    let mut recorder = Recorder::default();
    // Captured at PTT press time so transcription resolves the correct ask.
    let mut pressed_for: Option<String> = None;
    let mut disambiguation_candidates: Vec<(String, String)> = Vec::new(); // (agent_id, agent_name)

    // Pre-open the CoreAudio stream so the first PTT press is instant.
    if let Err(e) = recorder.warm() {
        tracing::warn!(error = %e, "could not pre-warm audio stream — first press may be slow");
    }

    loop {
        // Block until a hotkey event arrives (no polling sleep needed).
        let Ok(event) = receiver.recv_timeout(std::time::Duration::from_secs(1)) else {
            continue;
        };
        if event.id != hotkey.id() {
            continue;
        }
        match event.state {
            HotKeyState::Pressed => {
                // If the key-up was lost (e.g. menu was open and swallowed it),
                // the recorder may still think it's recording. Reset before starting fresh.
                if recorder.is_recording() {
                    recorder.stop();
                    state.recording.store(false, Ordering::Relaxed);
                    tracing::warn!("PTT: discarding stuck recording (key-up was lost)");
                }
                // Snapshot the pending ask NOW so transcription can't race
                // against a new ask that arrives while Whisper is running.
                let maybe_active = state.active_agent.lock().unwrap().clone();
                let router = state.router.blocking_lock();
                let agents = state.agents.blocking_read();
                let validated_active =
                    maybe_active.filter(|id| router.pending_question(id).is_some());

                if let Some(id) = validated_active {
                    pressed_for = Some(id);
                    disambiguation_candidates = Vec::new();
                } else {
                    // Stale or unset — clear active_agent and collect all pending for voice disambiguation
                    drop(router);
                    drop(agents);
                    *state.active_agent.lock().unwrap() = None;
                    let router = state.router.blocking_lock();
                    let agents = state.agents.blocking_read();
                    pressed_for = None;
                    disambiguation_candidates = router
                        .pending_agent_ids()
                        .map(|id| (id.to_string(), agents.name(id)))
                        .collect();
                }
                // Interrupt any ongoing TTS before we start listening.
                state.tts_kill.notify_one();
                tracing::info!(target_ask = ?pressed_for, "PTT pressed — listening");
                if let Err(e) = recorder.start() {
                    tracing::error!(error = %e, "failed to start recorder");
                    pressed_for = None;
                } else {
                    state.recording.store(true, Ordering::Relaxed);
                    // "Recording started" earcon — confirms the keypress registered.
                    #[cfg(target_os = "macos")]
                    std::process::Command::new("afplay")
                        .arg("/System/Library/Sounds/Tink.aiff")
                        .spawn()
                        .ok();
                }
            }
            HotKeyState::Released if recorder.is_recording() => {
                state.recording.store(false, Ordering::Relaxed);
                let audio = recorder.stop();
                let target = pressed_for.take();
                let candidates = std::mem::take(&mut disambiguation_candidates);
                if audio.len() < MIN_SAMPLES {
                    tracing::warn!("PTT release: too short, ignoring");
                } else {
                    handle_audio(&state, &transcriber, audio, target, candidates);
                }
            }
            _ => {}
        }
    }
}

fn handle_audio(
    state: &AppState,
    transcriber: &Transcriber,
    audio: Vec<f32>,
    target_id: Option<String>,
    candidates: Vec<(String, String)>,
) {
    let initial_prompt = if let Some(id) = target_id.as_deref() {
        let terms = state.agents.blocking_read().context_terms(id);
        state.glossary.whisper_prompt(&terms)
    } else if !candidates.is_empty() {
        // Prime Whisper with agent names so they transcribe accurately
        candidates
            .iter()
            .map(|(_, n)| n.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        String::new()
    };

    let transcript = match transcriber.transcribe(&audio, &initial_prompt) {
        Ok(t) if t.is_empty() => {
            tracing::info!("PTT: no speech detected");
            return;
        }
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "transcription failed");
            return;
        }
    };

    tracing::info!(transcript = %transcript, "transcribed");

    // Play earcon and pulse the processed badge for one tray-update cycle.
    #[cfg(target_os = "macos")]
    std::process::Command::new("afplay")
        .arg("/System/Library/Sounds/Glass.aiff")
        .spawn()
        .ok();
    state.just_processed.store(true, Ordering::Relaxed);

    let corrected = state.glossary.apply_corrections(&transcript);

    let (final_target, answer) = if let Some(id) = target_id {
        // Definite target: tray pre-selected or single pending agent
        (id, corrected)
    } else if candidates.len() == 1 {
        (candidates.into_iter().next().unwrap().0, corrected)
    } else if !candidates.is_empty() {
        // Voice disambiguation: user says "AgentName answer text"
        match extract_agent_prefix(&corrected, &candidates) {
            Some((id, ans)) if !ans.is_empty() => (id, ans),
            Some((id, _)) => {
                // User said just the agent name — confirm and wait for next press
                let name = candidates
                    .iter()
                    .find(|(cid, _)| *cid == id)
                    .map(|(_, n)| n.as_str())
                    .unwrap_or("agent");
                *state.active_agent.lock().unwrap() = Some(id);
                let _ = state
                    .tts_tx
                    .blocking_send(format!("Answering {}. Hold to reply.", name));
                return;
            }
            None => {
                let names: Vec<&str> = candidates.iter().map(|(_, n)| n.as_str()).collect();
                let _ = state
                    .tts_tx
                    .blocking_send(format!("Say the agent name first: {}", names.join(" or ")));
                return;
            }
        }
    } else {
        tracing::info!(transcript = %corrected, "PTT: no pending ask at press time, discarding");
        return;
    };

    let mut router = state.router.blocking_lock();
    if router.resolve(&final_target, answer.clone()) {
        drop(router);
        *state.active_agent.lock().unwrap() = None;
        tracing::info!(agent_id = %final_target, answer = %answer, "ask resolved via PTT");
    } else {
        tracing::info!(agent_id = %final_target, "PTT: ask already resolved or timed out");
    }
}

/// Extract an agent name prefix from a transcript for voice disambiguation.
/// Tries exact prefix match (longest name first), then fuzzy Jaro-Winkler.
/// Returns `(agent_id, remainder_answer)` on match.
fn extract_agent_prefix(
    transcript: &str,
    candidates: &[(String, String)],
) -> Option<(String, String)> {
    let t = transcript.trim().to_lowercase();

    // Longest name first to avoid "cursor" shadowing "cursor agent"
    let mut sorted: Vec<&(String, String)> = candidates.iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1.len()));

    // Exact prefix match
    for (id, name) in &sorted {
        let n = name.to_lowercase();
        if let Some(rest) = t.strip_prefix(n.as_str()) {
            let answer =
                rest.trim_start_matches(|c: char| c == ':' || c == ',' || c.is_whitespace());
            return Some((id.clone(), answer.to_string()));
        }
    }

    // Fuzzy: compare word-aligned prefix of transcript against agent name
    for (id, name) in &sorted {
        let n = name.to_lowercase();
        let word_count = n.split_whitespace().count();
        let t_prefix: String = t
            .split_whitespace()
            .take(word_count)
            .collect::<Vec<_>>()
            .join(" ");
        if strsim::jaro_winkler(&t_prefix, &n) > 0.85 {
            let answer: String = t
                .split_whitespace()
                .skip(word_count)
                .collect::<Vec<_>>()
                .join(" ");
            let answer = answer.trim_start_matches([':', ',']).trim().to_string();
            return Some((id.clone(), answer));
        }
    }

    None
}

fn parse_hotkey(s: &str) -> anyhow::Result<HotKey> {
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let (mod_parts, key_parts) = parts.split_at(parts.len().saturating_sub(1));

    let mut modifiers = Modifiers::empty();
    for m in mod_parts {
        match m.to_uppercase().as_str() {
            "ALT" | "OPTION" | "OPT" => modifiers |= Modifiers::ALT,
            "CTRL" | "CONTROL" => modifiers |= Modifiers::CONTROL,
            "SHIFT" => modifiers |= Modifiers::SHIFT,
            "META" | "CMD" | "COMMAND" | "SUPER" => modifiers |= Modifiers::META,
            m => anyhow::bail!("unknown modifier: {m}"),
        }
    }

    let code = parse_code(key_parts.first().copied().unwrap_or(""))?;
    Ok(HotKey::new(
        if modifiers.is_empty() {
            None
        } else {
            Some(modifiers)
        },
        code,
    ))
}

fn parse_code(s: &str) -> anyhow::Result<Code> {
    Ok(match s.to_uppercase().as_str() {
        "SPACE" => Code::Space,
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        "A" => Code::KeyA,
        "B" => Code::KeyB,
        "C" => Code::KeyC,
        "D" => Code::KeyD,
        "E" => Code::KeyE,
        "F" => Code::KeyF,
        "G" => Code::KeyG,
        "H" => Code::KeyH,
        "I" => Code::KeyI,
        "J" => Code::KeyJ,
        "K" => Code::KeyK,
        "L" => Code::KeyL,
        "M" => Code::KeyM,
        "N" => Code::KeyN,
        "O" => Code::KeyO,
        "P" => Code::KeyP,
        "Q" => Code::KeyQ,
        "R" => Code::KeyR,
        "S" => Code::KeyS,
        "T" => Code::KeyT,
        "U" => Code::KeyU,
        "V" => Code::KeyV,
        "W" => Code::KeyW,
        "X" => Code::KeyX,
        "Y" => Code::KeyY,
        "Z" => Code::KeyZ,
        k => anyhow::bail!("unknown key: \"{k}\". Use F1–F12, A–Z, or Space"),
    })
}
