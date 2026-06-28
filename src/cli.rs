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
