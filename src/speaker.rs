use tracing::info;

pub async fn speak(text: &str) {
    info!("TTS: {}", text);

    #[cfg(target_os = "macos")]
    {
        let _ = tokio::process::Command::new("say")
            .arg(text)
            .status()
            .await;
    }

    #[cfg(target_os = "linux")]
    {
        // Piper support to be wired in from config
        info!("TTS stub (Linux): {}", text);
    }
}
