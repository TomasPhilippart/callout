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
    /// Print the most recent log lines
    Logs {
        /// Number of lines to print
        #[arg(short = 'n', long, default_value = "100")]
        lines: usize,
    },
    /// Send a one-off spoken notification
    Notify {
        message: String,
        /// Explicit agent_id to attach to (default: reuses a session keyed by
        /// $CLAUDE_SESSION_ID, or "manual" if unset)
        #[arg(long)]
        agent_id: Option<String>,
    },
    /// Ask a yes/no or multiple-choice question and wait for a PTT answer
    Ask {
        question: String,
        /// Choice in "key:label" form, repeatable
        #[arg(long = "choice")]
        choices: Vec<String>,
        #[arg(long, default_value_t = crate::hook::DEFAULT_ASK_TIMEOUT_SECS)]
        timeout: u64,
        #[arg(long)]
        default: Option<String>,
        #[arg(long)]
        agent_id: Option<String>,
    },
    /// Claude Code hook entry points (read hook JSON from stdin)
    Hook {
        #[command(subcommand)]
        cmd: HookCmd,
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

#[derive(Subcommand)]
pub enum HookCmd {
    /// SessionStart hook: registers this session as an agent
    SessionStart,
    /// PreToolUse hook: voice-approve/deny the pending tool call
    PreToolUse,
    /// Stop hook: notify that the agent finished a turn
    Stop,
}
