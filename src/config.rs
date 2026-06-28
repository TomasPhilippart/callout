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
}

#[derive(Debug, Clone, Deserialize)]
pub struct HotkeyConfig {
    #[serde(default = "default_key")]
    pub key: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TtsConfig {
    pub piper_bin: Option<String>,
    pub piper_voice: Option<String>,
}

fn default_port() -> u16 { 7878 }
fn default_model() -> String { "base".into() }
fn default_key() -> String { "Alt".into() }

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            model: default_model(),
            hotkey: HotkeyConfig::default(),
            tts: TtsConfig::default(),
        }
    }
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self { key: default_key() }
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

    pub fn models_dir() -> PathBuf {
        Self::dir().join("models")
    }
}
