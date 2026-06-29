use crate::{cli::ModelCmd, Config};
use anyhow::{bail, Result};

const SIZES: &[&str] = &["tiny", "base", "small", "medium", "large"];

pub fn run(cmd: ModelCmd) -> Result<()> {
    match cmd {
        ModelCmd::Download { size } => cmd_download(&size),
        ModelCmd::List => cmd_list(),
    }
}

fn cmd_download(size: &str) -> Result<()> {
    if !SIZES.contains(&size) {
        bail!("unknown model size \"{size}\". Valid: {}", SIZES.join(", "));
    }

    let dest = Config::model_path(size);
    if dest.exists() {
        println!("Model already downloaded: {}", dest.display());
        return Ok(());
    }

    std::fs::create_dir_all(Config::models_dir())?;

    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{size}.bin");

    println!("Downloading ggml-{size}.bin → {}", dest.display());
    println!("(this may take a minute)\n");

    let status = std::process::Command::new("curl")
        .args(["-L", "--progress-bar", "-o"])
        .arg(&dest)
        .arg(&url)
        .status()?;

    if !status.success() {
        std::fs::remove_file(&dest).ok();
        bail!("download failed (curl exited {})", status);
    }

    println!("\nModel ready. Start callout to enable PTT transcription.");
    Ok(())
}

fn cmd_list() -> Result<()> {
    let active = Config::load().unwrap_or_default().model;
    println!("Whisper models in {}:\n", Config::models_dir().display());

    let mut found = false;
    for size in SIZES {
        let path = Config::model_path(size);
        if path.exists() {
            let mb = std::fs::metadata(&path)
                .map(|m| m.len() / 1_048_576)
                .unwrap_or(0);
            let tag = if *size == active { "  ← active" } else { "" };
            println!("  {size:<8} {mb:>4} MB{tag}");
            found = true;
        }
    }

    if !found {
        println!("  (none — run 'callout model download' to get one)");
    }

    println!("\nUsage:");
    println!("  callout model download base    # recommended starting point");
    println!("  callout model download small   # better accuracy, ~3× slower");
    Ok(())
}
