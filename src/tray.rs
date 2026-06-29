use std::sync::atomic::Ordering;

use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};

use crate::{agents::AgentState, AppState};

pub struct Tray {
    icon: TrayIcon,
    quit_id: MenuId,
    ptt_label: String,
}

pub fn build(ptt_key: &str) -> anyhow::Result<Tray> {
    let ptt_label = format_hotkey(ptt_key);
    let quit = MenuItem::new("Quit callout", true, None);
    let quit_id = quit.id().clone();

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(initial_menu(&ptt_label, &quit)))
        .with_tooltip("callout")
        .with_icon(mic_icon())
        .with_icon_as_template(true)
        .build()?;

    Ok(Tray {
        icon: tray,
        quit_id,
        ptt_label,
    })
}

/// Poll menu events; returns true if the user chose Quit.
pub fn poll(tray: &Tray) -> bool {
    while let Ok(event) = MenuEvent::receiver().try_recv() {
        if event.id == tray.quit_id {
            return true;
        }
    }
    false
}

/// Update only the icon and tooltip — reads only atomics, no locks.
/// Called every event-loop tick (~50 ms) for near-instant visual feedback.
pub fn update_icon(tray: &Tray, state: &AppState) {
    let recording = state.recording.load(Ordering::Relaxed);
    let just_processed = state
        .just_processed
        .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok();
    let speaking = state.tts_speaking.load(Ordering::Relaxed);

    // set_icon_as_template must be re-asserted after every set_icon call —
    // tray-icon 0.19 does not preserve it across swaps.
    let (icon, template, tooltip) = if recording {
        (recording_icon(), false, "callout — recording")
    } else if just_processed {
        (processed_icon(), false, "callout — heard you")
    } else if speaking {
        (speaking_icon(), false, "callout — speaking")
    } else {
        (mic_icon(), true, "callout")
    };

    tray.icon.set_icon(Some(icon)).ok();
    tray.icon.set_icon_as_template(template);
    tray.icon.set_tooltip(Some(tooltip)).ok();
}

/// Rebuild the full menu with live agent list.
/// Called every ~500 ms — takes brief locks on agents and router.
pub fn update_menu(tray: &Tray, state: &AppState) {
    let recording = state.recording.load(Ordering::Relaxed);

    let agents = state.agents.blocking_read();
    let router = state.router.blocking_lock();
    let agent_rows: Vec<(String, AgentState, Option<String>)> = agents
        .all()
        .iter()
        .map(|a| {
            let question = router
                .pending_question(&a.id)
                .map(|q| truncate(q, 30).to_string());
            (a.name.clone(), a.state.clone(), question)
        })
        .collect();
    drop(router);
    drop(agents);

    let quit = MenuItem::new("Quit callout", true, None);
    let menu = Menu::new();

    if recording {
        menu.append(&MenuItem::new("● Recording…", false, None))
            .ok();
        menu.append(&PredefinedMenuItem::separator()).ok();
    }

    if agent_rows.is_empty() {
        menu.append(&MenuItem::new("No agents connected", false, None))
            .ok();
    } else {
        for (name, state, question) in &agent_rows {
            let label = match (state, question) {
                (AgentState::Waiting, Some(q)) => format!("{name}  ⏳ {q}"),
                (AgentState::Waiting, None) => format!("{name}  ⏳"),
                _ => name.clone(),
            };
            menu.append(&MenuItem::new(label, false, None)).ok();
        }
    }

    menu.append(&PredefinedMenuItem::separator()).ok();
    menu.append(&MenuItem::new(
        format!("Hold {} to speak", tray.ptt_label),
        false,
        None,
    ))
    .ok();
    menu.append(&PredefinedMenuItem::separator()).ok();
    menu.append(&quit).ok();

    tray.icon.set_menu(Some(Box::new(menu)));
}

// Keep the combined fn for any callers that want both at once.
pub fn update(tray: &Tray, state: &AppState) {
    update_menu(tray, state);

    let recording = state.recording.load(Ordering::Relaxed);
    let just_processed = state
        .just_processed
        .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok();
    let speaking = state.tts_speaking.load(Ordering::Relaxed);

    let (icon, template, tooltip) = if recording {
        (recording_icon(), false, "callout — recording")
    } else if just_processed {
        (processed_icon(), false, "callout — heard you")
    } else if speaking {
        (speaking_icon(), false, "callout — speaking")
    } else {
        (mic_icon(), true, "callout")
    };

    tray.icon.set_icon(Some(icon)).ok();
    tray.icon.set_icon_as_template(template);
    tray.icon.set_tooltip(Some(tooltip)).ok();
}

fn initial_menu(ptt_label: &str, quit: &MenuItem) -> Menu {
    let menu = Menu::new();
    menu.append(&MenuItem::new("No agents connected", false, None))
        .ok();
    menu.append(&PredefinedMenuItem::separator()).ok();
    menu.append(&MenuItem::new(
        format!("Hold {ptt_label} to speak"),
        false,
        None,
    ))
    .ok();
    menu.append(&PredefinedMenuItem::separator()).ok();
    menu.append(quit).ok();
    menu
}

/// Truncate to at most `max_chars` chars, appending "…" if cut.
fn truncate(s: &str, max_chars: usize) -> &str {
    if s.chars().count() <= max_chars {
        s
    } else {
        // Find byte position of the max_chars-th char boundary
        s.char_indices()
            .nth(max_chars)
            .map(|(i, _)| &s[..i])
            .unwrap_or(s)
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
