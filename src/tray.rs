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

/// Rebuild the menu and icon to reflect current agent + recording state.
/// Called periodically from the main-thread event loop (~500 ms).
pub fn update(tray: &Tray, state: &AppState) {
    let recording = state.recording.load(Ordering::Relaxed);

    // Build agent rows — read locks are brief
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

    // Rebuild menu
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

    // Swap icon and tooltip based on recording state
    if recording {
        tray.icon.set_icon(Some(recording_icon())).ok();
        tray.icon.set_tooltip(Some("callout — recording")).ok();
    } else {
        tray.icon.set_icon(Some(mic_icon())).ok();
        tray.icon.set_tooltip(Some("callout")).ok();
    }
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

/// Template mic icon — macOS inverts it automatically for dark/light mode.
fn mic_icon() -> Icon {
    const W: usize = 22;
    const H: usize = 22;
    let mut on = [[false; W]; H];

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

    let rgba: Vec<u8> = on
        .iter()
        .flatten()
        .flat_map(|&p| if p { [0u8, 0, 0, 255] } else { [0u8, 0, 0, 0] })
        .collect();

    Icon::from_rgba(rgba, W as u32, H as u32).expect("valid icon dimensions")
}

/// Solid red circle — NOT a template so it stays red in both modes.
fn recording_icon() -> Icon {
    const W: usize = 22;
    const H: usize = 22;
    let cx = W as f32 / 2.0;
    let cy = H as f32 / 2.0;

    let rgba: Vec<u8> = (0..H)
        .flat_map(|y| {
            (0..W).flat_map(move |x| {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                if (dx * dx + dy * dy).sqrt() < 8.0 {
                    [220u8, 38, 38, 255] // red-600
                } else {
                    [0u8, 0, 0, 0]
                }
            })
        })
        .collect();

    Icon::from_rgba(rgba, W as u32, H as u32).expect("valid icon dimensions")
}
