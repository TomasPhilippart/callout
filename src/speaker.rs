pub async fn speak(text: &str, voice: &str) {
    tracing::info!(text = %text, voice = %voice, "speaking");

    #[cfg(target_os = "macos")]
    match tokio::process::Command::new("say")
        .arg("-v")
        .arg(voice)
        .arg(text)
        .status()
        .await
    {
        Ok(status) if !status.success() => {
            tracing::warn!(text = %text, %status, "say exited non-zero");
        }
        Err(e) => {
            tracing::error!(text = %text, error = %e, "failed to invoke say");
        }
        _ => {}
    }

    #[cfg(target_os = "linux")]
    tracing::info!(text = %text, "TTS stub (Linux piper not yet configured)");
}
