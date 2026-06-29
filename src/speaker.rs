use std::sync::Arc;
use tokio::sync::Notify;

pub async fn speak(text: &str, voice: &str, kill: &Arc<Notify>) {
    tracing::info!(text = %text, voice = %voice, "speaking");

    #[cfg(target_os = "macos")]
    {
        let mut child = match tokio::process::Command::new("say")
            .arg("-v")
            .arg(voice)
            .arg(text)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(text = %text, error = %e, "failed to spawn say");
                return;
            }
        };

        tokio::select! {
            status = child.wait() => {
                match status {
                    Ok(s) if !s.success() => tracing::warn!(text = %text, %s, "say exited non-zero"),
                    Err(e) => tracing::error!(text = %text, error = %e, "say wait error"),
                    _ => {}
                }
            }
            _ = kill.notified() => {
                child.start_kill().ok();
                child.wait().await.ok();
                tracing::info!("TTS interrupted by barge-in");
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let _ = kill;
        tracing::info!(text = %text, "TTS stub (Linux piper not yet configured)");
    }
}
