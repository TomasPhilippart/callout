use anyhow::Context;
use std::path::PathBuf;

use crate::Config;

/// Find the most recently-rotated log file (tracing-appender's daily files
/// sort lexicographically by date, so the max filename is the newest).
fn latest_log_file() -> anyhow::Result<Option<PathBuf>> {
    let dir = Config::logs_dir();
    if !dir.exists() {
        return Ok(None);
    }

    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("failed to read log directory {}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("callout.log."))
        })
        .collect();

    candidates.sort();
    Ok(candidates.into_iter().next_back())
}

pub fn print_recent(lines: usize) -> anyhow::Result<()> {
    let Some(path) = latest_log_file()? else {
        println!("no logs found — has callout been run yet?");
        return Ok(());
    };

    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read log file {}", path.display()))?;

    let all_lines: Vec<&str> = contents.lines().collect();
    let start = all_lines.len().saturating_sub(lines);

    println!("== {} ==", path.display());
    for line in &all_lines[start..] {
        println!("{line}");
    }

    Ok(())
}
