use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    TrayIcon, TrayIconBuilder,
};

/// Handle to the tray icon; dropped when the daemon exits.
pub struct Tray {
    _icon: TrayIcon,
    pub quit_id: MenuId,
}

pub fn build(ptt_key: &str) -> anyhow::Result<Tray> {
    let hotkey_label = format_hotkey(ptt_key);

    let info = MenuItem::new(format!("Hold {hotkey_label} to speak"), false, None);
    let quit = MenuItem::new("Quit callout", true, None);
    let quit_id = quit.id().clone();

    let menu = Menu::new();
    menu.append(&info)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit)?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("callout")
        .with_icon(mic_icon())
        .with_icon_as_template(true)
        .build()?;

    Ok(Tray {
        _icon: tray,
        quit_id,
    })
}

/// Poll the menu channel and return true if the user chose Quit.
pub fn poll(tray: &Tray) -> bool {
    while let Ok(event) = MenuEvent::receiver().try_recv() {
        if event.id == tray.quit_id {
            return true;
        }
    }
    false
}

/// Convert a hotkey string like "Alt+K" → "⌥K" for display.
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

/// 22×22 microphone silhouette as a template icon (black + alpha).
fn mic_icon() -> tray_icon::Icon {
    const W: usize = 22;
    const H: usize = 22;
    let mut on = [[false; W]; H];

    // Capsule body: rows 2–11, cols 7–14 (clipped top corners)
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
    // Stand arms: cols 7 and 14, rows 12–15
    on.iter_mut().skip(12).take(4).for_each(|row| {
        row[7] = true;
        row[14] = true;
    });
    // Stand bottom: row 15, cols 7–14
    on[15].iter_mut().skip(7).take(8).for_each(|px| *px = true);
    // Stem: cols 10–11, rows 16–17
    on.iter_mut().skip(16).take(2).for_each(|row| {
        row[10] = true;
        row[11] = true;
    });
    // Base: row 18, cols 8–13
    on[18].iter_mut().skip(8).take(6).for_each(|px| *px = true);

    let rgba: Vec<u8> = on
        .iter()
        .flatten()
        .flat_map(|&p| if p { [0u8, 0, 0, 255] } else { [0u8, 0, 0, 0] })
        .collect();

    tray_icon::Icon::from_rgba(rgba, W as u32, H as u32).expect("valid icon dimensions")
}
