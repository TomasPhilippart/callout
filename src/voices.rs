use anyhow::{bail, Result};
use toml_edit::{value, DocumentMut};
use crate::{cli::VoicesCmd, Config};

pub struct Voice {
    pub name: String,
    pub locale: String,
}

pub fn run(cmd: VoicesCmd) -> Result<()> {
    match cmd {
        VoicesCmd::List      => cmd_list(),
        VoicesCmd::Set {name} => cmd_set(name),
        VoicesCmd::Download  => cmd_download(),
    }
}

fn cmd_list() -> Result<()> {
    let voices = installed();
    let current = Config::load().unwrap_or_default().tts.voice;

    let mut en: Vec<&Voice> = voices.iter().filter(|v| v.locale.starts_with("en_")).collect();
    let mut other: Vec<&Voice> = voices.iter().filter(|v| !v.locale.starts_with("en_")).collect();

    en.sort_by(|a, b| a.name.cmp(&b.name));
    other.sort_by(|a, b| a.locale.cmp(&b.locale).then(a.name.cmp(&b.name)));

    println!("English voices:");
    for v in &en {
        let tag = if v.name == current { "  ← active" } else { "" };
        println!("  {:<32} {}{}", v.name, v.locale, tag);
    }
    if !other.is_empty() {
        println!("\nOther languages:");
        for v in &other {
            let tag = if v.name == current { "  ← active" } else { "" };
            println!("  {:<32} {}{}", v.name, v.locale, tag);
        }
    }
    println!("\nUsage:");
    println!("  callout voices set \"Ava (Premium)\"");
    println!("  callout voices download      # open System Settings to get more");
    Ok(())
}

fn cmd_set(name: String) -> Result<()> {
    if !installed().iter().any(|v| v.name == name) {
        bail!(
            "Voice \"{name}\" is not installed.\n\
             Run 'callout voices list' to see what's available, or\n\
             'callout voices download' to get more."
        );
    }
    write_voice(&name)?;
    println!("Voice set to \"{name}\". Restart callout to apply.");
    Ok(())
}

fn cmd_download() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.universalaccess?spoken")
            .spawn()?;
        println!("Opened System Settings → Accessibility → Spoken Content.");
        println!("Download a Premium voice, then run 'callout voices list' to confirm.");
    }
    #[cfg(not(target_os = "macos"))]
    {
        println!("On Linux, install piper-tts: https://github.com/rhasspy/piper");
        println!("Then set piper_bin and piper_voice in ~/.callout/config.toml");
    }
    Ok(())
}

pub fn installed() -> Vec<Voice> {
    let Ok(out) = std::process::Command::new("say")
        .arg("-v").arg("?")
        .output() else { return vec![] };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(parse_line)
        .collect()
}

pub fn is_installed(name: &str) -> bool {
    installed().iter().any(|v| v.name == name)
}

fn parse_line(line: &str) -> Option<Voice> {
    // "Ava (Premium)       en_US    # Hello! My name is Ava."
    let before = line[..line.find('#')?].trim();
    let mut tokens: Vec<&str> = before.split_whitespace().collect();
    let locale = tokens.pop()?.to_string();
    if !locale.contains('_') { return None; }
    let name = tokens.join(" ");
    if name.is_empty() { return None; }
    Some(Voice { name, locale })
}

fn write_voice(voice: &str) -> Result<()> {
    let dir = Config::dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("config.toml");

    let content = if path.exists() { std::fs::read_to_string(&path)? } else { String::new() };
    let mut doc: DocumentMut = content.parse()?;

    if !doc.contains_key("tts") {
        doc["tts"] = toml_edit::table();
    }
    doc["tts"]["voice"] = value(voice);

    std::fs::write(&path, doc.to_string())?;
    Ok(())
}
