use std::sync::{atomic::Ordering, Arc};

use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};

use crate::{recorder::Recorder, transcriber::Transcriber, AppState};

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
            HotKeyState::Pressed if !recorder.is_recording() => {
                // Snapshot the pending ask NOW so transcription can't race
                // against a new ask that arrives while Whisper is running.
                pressed_for = state
                    .router
                    .blocking_lock()
                    .pending_agent_ids()
                    .next()
                    .map(str::to_string);
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
                if audio.len() < 1600 {
                    tracing::warn!("PTT release: too short, ignoring");
                } else {
                    handle_audio(&state, &transcriber, audio, target);
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
) {
    let initial_prompt = target_id
        .as_deref()
        .map(|id| {
            let terms = state.agents.blocking_read().context_terms(id);
            state.glossary.whisper_prompt(&terms)
        })
        .unwrap_or_default();

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

    // Play earcon so the user knows their speech was received.
    #[cfg(target_os = "macos")]
    std::process::Command::new("afplay")
        .arg("/System/Library/Sounds/Glass.aiff")
        .spawn()
        .ok();

    let corrected = state.glossary.apply_corrections(&transcript);

    let mut router = state.router.blocking_lock();
    if let Some(id) = target_id {
        if router.resolve(&id, corrected.clone()) {
            tracing::info!(agent_id = %id, answer = %corrected, "ask resolved via PTT");
        } else {
            tracing::info!(agent_id = %id, "PTT: ask already resolved or timed out");
        }
    } else {
        tracing::info!(transcript = %corrected, "PTT: no pending ask at press time, discarding");
    }
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
