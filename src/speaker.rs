use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Notify;

pub async fn speak(text: &str, voice: &str, kill: &Arc<Notify>, speaking: &Arc<AtomicBool>) {
    // Drain any stale permit left by a PTT press that happened while nothing
    // was speaking — otherwise the next speak() would be killed immediately.
    tokio::time::timeout(std::time::Duration::ZERO, kill.notified())
        .await
        .ok();

    tracing::info!(text = %text, voice = %voice, "speaking");

    #[cfg(target_os = "macos")]
    {
        speaking.store(true, Ordering::Relaxed);

        // "Attention" earcon — plays before speech so the user knows to listen.
        // speaking is already true so the tray shows the orange dot during the earcon.
        let _ = std::process::Command::new("afplay")
            .arg("/System/Library/Sounds/Purr.aiff")
            .status();

        let mut child = match tokio::process::Command::new("say")
            .arg("-v")
            .arg(voice)
            .arg(text)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(text = %text, error = %e, "failed to spawn say");
                speaking.store(false, Ordering::Relaxed);
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

        speaking.store(false, Ordering::Relaxed);
    }

    #[cfg(target_os = "linux")]
    {
        let _ = (kill, speaking);
        tracing::info!(text = %text, "TTS stub (Linux piper not yet configured)");
    }
}
