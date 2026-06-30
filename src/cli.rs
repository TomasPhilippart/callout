use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "callout", about = "Ambient voice for AI agents", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start the HTTP daemon (default when no command given)
    Serve,
    /// Manage TTS voices
    Voices {
        #[command(subcommand)]
        cmd: VoicesCmd,
    },
    /// Manage Whisper models
    Model {
        #[command(subcommand)]
        cmd: ModelCmd,
    },
    /// Debug: show configured PTT hotkey
    PttTest,
    /// Register callout as a login item (macOS only)
    #[cfg(target_os = "macos")]
    Install,
    /// Remove callout from login items (macOS only)
    #[cfg(target_os = "macos")]
    Uninstall,
}

#[derive(Subcommand)]
pub enum VoicesCmd {
    /// List installed voices
    List,
    /// Set the active voice and save to config
    Set {
        /// Voice name exactly as shown in 'voices list', e.g. "Ava (Premium)"
        name: String,
    },
    /// Open System Settings to download more voices
    Download,
}

#[derive(Subcommand)]
pub enum ModelCmd {
    /// Download a Whisper model
    Download {
        /// Model size: tiny (~75 MB), base (~148 MB), small (~488 MB)
        #[arg(default_value = "base")]
        size: String,
    },
    /// Show downloaded models and their paths
    List,
}
