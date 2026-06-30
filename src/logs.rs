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
        .filter_map(|entry| match entry {
            Ok(e) => Some(e.path()),
            Err(e) => {
                eprintln!("warning: skipping unreadable log directory entry: {e}");
                None
            }
        })
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("callout.log."))
        })
        .collect();

    candidates.sort();
    Ok(candidates.into_iter().next_back())
}

fn tail_lines(contents: &str, n: usize) -> Vec<&str> {
    let all_lines: Vec<&str> = contents.lines().collect();
    let start = all_lines.len().saturating_sub(n);
    all_lines[start..].to_vec()
}

pub fn print_recent(lines: usize) -> anyhow::Result<()> {
    let Some(path) = latest_log_file()? else {
        println!("no logs found — has callout been run yet?");
        return Ok(());
    };

    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read log file {}", path.display()))?;

    println!("== {} ==", path.display());
    for line in tail_lines(&contents, lines) {
        println!("{line}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_lines_returns_all_when_fewer_than_n() {
        let contents = "a\nb\nc";
        assert_eq!(tail_lines(contents, 10), vec!["a", "b", "c"]);
    }

    #[test]
    fn tail_lines_truncates_to_last_n() {
        let contents = "a\nb\nc\nd\ne";
        assert_eq!(tail_lines(contents, 2), vec!["d", "e"]);
    }

    #[test]
    fn tail_lines_zero_returns_empty() {
        let contents = "a\nb\nc";
        assert_eq!(tail_lines(contents, 0), Vec::<&str>::new());
    }

    #[test]
    fn tail_lines_empty_contents_returns_empty() {
        assert_eq!(tail_lines("", 5), Vec::<&str>::new());
    }
}
