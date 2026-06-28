use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub hotkey: HotkeyConfig,
    #[serde(default)]
    pub tts: TtsConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HotkeyConfig {
    #[serde(default = "default_key")]
    pub key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TtsConfig {
    /// macOS: name passed to `say -v`. Set to "Ava (Premium)" for best quality.
    #[serde(default = "default_voice")]
    pub voice: String,
    /// Linux: path to piper binary.
    pub piper_bin: Option<String>,
    /// Linux: path to piper voice model.
    pub piper_voice: Option<String>,
}

fn default_voice() -> String { "Samantha".into() }

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            voice: default_voice(),
            piper_bin: None,
            piper_voice: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DaemonConfig {
    /// How often (in seconds) the agent prune job runs.
    #[serde(default = "default_prune_interval_secs")]
    pub prune_interval_secs: u64,
    /// Agents not seen within this many seconds are considered stale.
    #[serde(default = "default_stale_secs")]
    pub stale_secs: u64,
}

fn default_port() -> u16 { 7878 }
fn default_model() -> String { "base".into() }
fn default_key() -> String { "Alt".into() }
fn default_prune_interval_secs() -> u64 { 60 }
fn default_stale_secs() -> u64 { 300 }

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            model: default_model(),
            hotkey: HotkeyConfig::default(),
            tts: TtsConfig::default(),
            daemon: DaemonConfig::default(),
        }
    }
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self { key: default_key() }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            prune_interval_secs: default_prune_interval_secs(),
            stale_secs: default_stale_secs(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Self::dir().join("config.toml");
        if path.exists() {
            let s = std::fs::read_to_string(&path)?;
            Ok(toml::from_str(&s)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn dir() -> PathBuf {
        dirs::home_dir().unwrap_or_default().join(".callout")
    }
}
