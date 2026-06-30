use anyhow::Context;
use std::path::PathBuf;

const LABEL: &str = "com.callout";
const PLIST_FILENAME: &str = "com.callout.plist";

fn home_dir() -> anyhow::Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))
}

fn plist_path() -> anyhow::Result<PathBuf> {
    Ok(home_dir()?
        .join("Library/LaunchAgents")
        .join(PLIST_FILENAME))
}

fn log_dir() -> PathBuf {
    crate::Config::logs_dir()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn plist_contents(binary: &str, stderr: &str) -> String {
    let binary = xml_escape(binary);
    let stderr = xml_escape(stderr);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardErrorPath</key>
    <string>{stderr}</string>
</dict>
</plist>
"#
    )
}

/// Shell out to `id -u` rather than depending on libc, since this runs once per
/// install/uninstall invocation — not a hot path.
fn uid() -> anyhow::Result<String> {
    let output = std::process::Command::new("id")
        .arg("-u")
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run `id -u`: {e}"))?;
    if !output.status.success() {
        anyhow::bail!("`id -u` exited with status {}", output.status);
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_string())
        .map_err(|e| anyhow::anyhow!("`id -u` output was not valid UTF-8: {e}"))
}

pub fn install() -> anyhow::Result<()> {
    let binary =
        std::env::current_exe().map_err(|e| anyhow::anyhow!("cannot resolve binary path: {e}"))?;
    let binary_str = binary
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("binary path is not valid UTF-8"))?;

    // Warn if the binary looks like a debug/temp build — installing that path
    // means the LaunchAgent breaks the moment the build artifact is cleaned.
    if binary_str.contains("/target/debug/") || binary_str.contains("/var/folders/") {
        eprintln!(
            "warning: binary is at {binary_str}\n\
             For a permanent install, run `cargo install --path .` first."
        );
    }

    let plist = plist_path()?;
    if plist.exists() {
        anyhow::bail!(
            "already installed ({}). Run `callout uninstall` first.",
            plist.display()
        );
    }

    let log_dir = log_dir();
    std::fs::create_dir_all(&log_dir)
        .with_context(|| format!("failed to create log directory {}", log_dir.display()))?;
    let stderr = log_dir.join("launchd-stderr.log");

    let contents = plist_contents(binary_str, stderr.to_str().unwrap());

    std::fs::write(&plist, &contents)
        .with_context(|| format!("failed to write plist to {}", plist.display()))?;
    println!("wrote {}", plist.display());

    // Load immediately: launchctl bootstrap gui/<uid> <plist>
    let uid = uid()?;
    let status = std::process::Command::new("launchctl")
        .args(["bootstrap", &format!("gui/{uid}"), plist.to_str().unwrap()])
        .status()?;

    if !status.success() {
        // Remove the plist so the user isn't left in a broken state
        let _ = std::fs::remove_file(&plist);
        anyhow::bail!("launchctl bootstrap failed (exit {})", status);
    }

    println!("callout will now start automatically at login.");
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    let plist = plist_path()?;

    if !plist.exists() {
        anyhow::bail!("not installed (plist not found at {})", plist.display());
    }

    // Unload: launchctl bootout gui/<uid> <plist>
    let uid = uid()?;
    let status = std::process::Command::new("launchctl")
        .args(["bootout", &format!("gui/{uid}"), plist.to_str().unwrap()])
        .status()?;

    if !status.success() {
        // Non-fatal: the service may not be running; still remove the plist
        eprintln!("warning: launchctl bootout exited with status {status} (continuing)");
    }

    std::fs::remove_file(&plist)
        .with_context(|| format!("failed to remove plist at {}", plist.display()))?;
    println!("callout removed from login items.");
    Ok(())
}
