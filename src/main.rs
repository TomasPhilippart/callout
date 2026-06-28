use clap::Parser;
use callout::cli::{Cli, Command};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None | Some(Command::Serve) => {
            tokio::runtime::Runtime::new()?.block_on(callout::run())
        }
        Some(Command::Voices { cmd }) => callout::voices::run(cmd),
    }
}
