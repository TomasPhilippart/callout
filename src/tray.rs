use std::{
    cell::{Cell, RefCell},
    sync::atomic::Ordering,
};

use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};

use crate::{agents::AgentState, AppState};

#[derive(Clone, Copy, PartialEq)]
enum IconState {
    Idle,
    Recording,
    Processed,
    Speaking,
}

pub struct Tray {
    icon: TrayIcon,
    quit_id: MenuId,
    ptt_label: String,
    /// Tracks last-applied icon state so set_icon is only called on transitions.
    last_icon: Cell<IconState>,
    /// Cached menu content — set_menu is only called when this changes.
    last_menu: RefCell<MenuSnapshot>,
}

#[derive(Default, PartialEq)]
struct MenuSnapshot {
    recording: bool,
    rows: Vec<(String, String, AgentState, Option<String>)>, // (agent_id, agent_name, state, question)
    active_agent_id: Option<String>,
}

pub fn build(ptt_key: &str) -> anyhow::Result<Tray> {
    let ptt_label = format_hotkey(ptt_key);
    let quit_id = MenuId::new("quit");

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(build_menu(
            &ptt_label,
            &quit_id,
            &MenuSnapshot::default(),
        )))
        .with_tooltip("callout")
        .with_icon(mic_icon())
        .with_icon_as_template(true)
        .build()?;

    Ok(Tray {
        icon: tray,
        quit_id,
        ptt_label,
        last_icon: Cell::new(IconState::Idle),
        last_menu: RefCell::new(MenuSnapshot::default()),
    })
}

/// Poll menu events; returns true if the user chose Quit.
pub fn poll(tray: &Tray, state: Option<&AppState>) -> bool {
    while let Ok(event) = MenuEvent::receiver().try_recv() {
        if event.id == tray.quit_id {
            return true;
        }
        if let Some(state) = state {
            if let Some(agent_id) = event.id.0.strip_prefix("agent:") {
                *state.active_agent.lock().unwrap() = Some(agent_id.to_string());
                tracing::info!(agent_id = %agent_id, "tray: active agent pre-selected");
            }
        }
    }
    false
}

/// Update only the icon and tooltip — reads only atomics, no locks.
/// Called every event-loop tick (~50 ms) but only issues AppKit calls when
/// the state actually changes, avoiding flicker and menu-click interference.
pub fn update_icon(tray: &Tray, state: &AppState) {
    let recording = state.recording.load(Ordering::Relaxed);
    let speaking = state.tts_speaking.load(Ordering::Relaxed);

    let new_state = if recording {
        IconState::Recording
    } else if speaking {
        IconState::Speaking
    } else if state
        .just_processed
        .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        IconState::Processed
    } else {
        IconState::Idle
    };

    if new_state == tray.last_icon.get() {
        return; // no transition — skip the AppKit calls entirely
    }
    tray.last_icon.set(new_state);

    // set_icon_as_template must be re-asserted after every set_icon call —
    // tray-icon 0.19 does not preserve it across swaps.
    let (icon, template, tooltip) = match new_state {
        IconState::Recording => (recording_icon(), false, "callout — recording"),
        IconState::Processed => (processed_icon(), false, "callout — heard you"),
        IconState::Speaking => (speaking_icon(), false, "callout — speaking"),
        IconState::Idle => (mic_icon(), true, "callout"),
    };

    tray.icon.set_icon(Some(icon)).ok();
    tray.icon.set_icon_as_template(template);
    tray.icon.set_tooltip(Some(tooltip)).ok();
}

/// Rebuild the full menu with live agent list.
/// Called every ~500 ms but only issues set_menu when content actually changed,
/// so an open menu is never replaced mid-interaction.
pub fn update_menu(tray: &Tray, state: &AppState) {
    let recording = state.recording.load(Ordering::Relaxed);

    let agents = state.agents.blocking_read();
    let router = state.router.blocking_lock();
    let rows: Vec<(String, String, AgentState, Option<String>)> = agents
        .all()
        .iter()
        .map(|a| {
            let question = router.pending_question(&a.id).map(|q| truncate(q, 30));
            (a.id.clone(), a.name.clone(), a.state.clone(), question)
        })
        .collect();
    drop(router);
    drop(agents);

    let active_agent_id = state.active_agent.lock().unwrap().clone();
    let snapshot = MenuSnapshot {
        recording,
        rows,
        active_agent_id,
    };
    if *tray.last_menu.borrow() == snapshot {
        return; // nothing changed — leave the menu alone
    }

    tray.icon.set_menu(Some(Box::new(build_menu(
        &tray.ptt_label,
        &tray.quit_id,
        &snapshot,
    ))));
    *tray.last_menu.borrow_mut() = snapshot;
}

/// Build a fresh menu from the given snapshot.
/// Uses the stable `quit_id` so Quit events always match `poll()`'s check.
fn build_menu(ptt_label: &str, quit_id: &MenuId, snapshot: &MenuSnapshot) -> Menu {
    let menu = Menu::new();

    if snapshot.recording {
        menu.append(&MenuItem::new("● Recording…", false, None))
            .ok();
        menu.append(&PredefinedMenuItem::separator()).ok();
    }

    if snapshot.rows.is_empty() {
        menu.append(&MenuItem::new("No agents connected", false, None))
            .ok();
    } else {
        for (agent_id, name, agent_state, question) in &snapshot.rows {
            let is_active = snapshot.active_agent_id.as_deref() == Some(agent_id.as_str());
            let is_pending = matches!(agent_state, AgentState::Waiting) && question.is_some();

            let prefix = if is_active { "→ " } else { "" };
            let label = match (agent_state, question) {
                (AgentState::Waiting, Some(q)) => format!("{prefix}{name}  ⏳ {q}"),
                (AgentState::Waiting, None) => format!("{prefix}{name}  ⏳"),
                _ => format!("{prefix}{name}"),
            };

            if is_pending {
                menu.append(&MenuItem::with_id(
                    MenuId::new(format!("agent:{agent_id}")),
                    label,
                    true,
                    None,
                ))
                .ok();
            } else {
                menu.append(&MenuItem::new(label, false, None)).ok();
            }
        }
    }

    menu.append(&PredefinedMenuItem::separator()).ok();
    menu.append(&MenuItem::new(
        format!("Hold {ptt_label} to speak"),
        false,
        None,
    ))
    .ok();
    menu.append(&PredefinedMenuItem::separator()).ok();
    // Reuse the stable quit_id so poll() can always recognise the Quit item.
    menu.append(&MenuItem::with_id(
        quit_id.clone(),
        "Quit callout",
        true,
        None,
    ))
    .ok();

    menu
}

/// Return `s` unchanged if ≤ `max_chars`, otherwise truncate and append "…".
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}…", &s[..end])
    }
}

/// "Alt+K" → "⌥K", "Ctrl+Shift+F5" → "⌃⇧F5"
fn format_hotkey(s: &str) -> String {
    s.split('+')
        .map(|part| match part.trim().to_uppercase().as_str() {
            "ALT" | "OPTION" | "OPT" => "⌥".to_string(),
            "CTRL" | "CONTROL" => "⌃".to_string(),
            "SHIFT" => "⇧".to_string(),
            "META" | "CMD" | "COMMAND" | "SUPER" => "⌘".to_string(),
            k => k.to_string(),
        })
        .collect()
}

// ── Icons ────────────────────────────────────────────────────────────────────

fn mic_pixels() -> [[bool; 22]; 22] {
    let mut on = [[false; 22usize]; 22usize];
    // Capsule body: rows 2–11, cols 7–14 (rounded top corners removed)
    on.iter_mut()
        .enumerate()
        .skip(2)
        .take(10)
        .for_each(|(r, row)| {
            row.iter_mut()
                .enumerate()
                .skip(7)
                .take(8)
                .for_each(|(c, px)| {
                    *px = !(r == 2 && (c == 7 || c == 14));
                });
        });
    // Stand arms
    on.iter_mut().skip(12).take(4).for_each(|row| {
        row[7] = true;
        row[14] = true;
    });
    // Stand bottom arc
    on[15].iter_mut().skip(7).take(8).for_each(|px| *px = true);
    // Stem
    on.iter_mut().skip(16).take(2).for_each(|row| {
        row[10] = true;
        row[11] = true;
    });
    // Base
    on[18].iter_mut().skip(8).take(6).for_each(|px| *px = true);
    on
}

/// Template mic icon — macOS inverts it automatically for dark/light mode.
fn mic_icon() -> Icon {
    let on = mic_pixels();
    let rgba: Vec<u8> = on
        .iter()
        .flatten()
        .flat_map(|&p| if p { [0u8, 0, 0, 255] } else { [0u8, 0, 0, 0] })
        .collect();
    Icon::from_rgba(rgba, 22, 22).expect("valid icon dimensions")
}

/// Mic with a small filled dot badge in the top-right corner.
/// The mic body is drawn in white (not template) so the badge color is preserved.
/// badge_rgba: [r, g, b, a] for the dot color.
fn mic_with_badge(badge_rgba: [u8; 4]) -> Icon {
    const W: usize = 22;
    const H: usize = 22;
    // Badge: circle centred at (17, 5) with radius 4.5
    const BX: f32 = 17.5;
    const BY: f32 = 4.5;
    const BR: f32 = 4.5;

    let on = mic_pixels();
    let rgba: Vec<u8> = (0..H)
        .flat_map(|y| {
            (0..W).flat_map(move |x| {
                let dx = x as f32 + 0.5 - BX;
                let dy = y as f32 + 0.5 - BY;
                if dx * dx + dy * dy < BR * BR {
                    badge_rgba
                } else if on[y][x] {
                    [255u8, 255, 255, 255] // white mic body (non-template)
                } else {
                    [0u8, 0, 0, 0]
                }
            })
        })
        .collect();
    Icon::from_rgba(rgba, W as u32, H as u32).expect("valid icon dimensions")
}

fn recording_icon() -> Icon {
    mic_with_badge([220, 38, 38, 255]) // red
}

fn speaking_icon() -> Icon {
    mic_with_badge([251, 146, 60, 255]) // orange
}

fn processed_icon() -> Icon {
    mic_with_badge([34, 197, 94, 255]) // green
}
