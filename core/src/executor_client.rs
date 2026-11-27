//! Read-only calls to the Aegis Executor (`/api/v1/diagnostics`) from the
//! host process. Mutating commands stay in `hitl.rs` (Ed25519-signed
//! `/api/v1/execute` after owner approval); this module only runs the
//! executor's read-only whitelist.

use std::sync::Mutex;
use std::time::{Duration, Instant};

fn executor_url() -> String {
    std::env::var("AEGIS_EXECUTOR_URL").unwrap_or_else(|_| "http://127.0.0.1:9876".to_string())
}

/// Run one whitelisted diagnostic command and return its stdout.
pub async fn diagnostics(command: &str, timeout: Duration) -> anyhow::Result<String> {
    let client = reqwest::Client::builder().timeout(timeout).build()?;
    let url = format!(
        "{}/api/v1/diagnostics?command={}&timeout_secs={}",
        executor_url(),
        urlencoding::encode(command),
        timeout.as_secs().clamp(1, 180)
    );
    let mut request = client.get(url);
    if let Ok(token) = std::env::var("AEGIS_EXECUTOR_TOKEN")
        && !token.trim().is_empty()
    {
        request = request.bearer_auth(token.trim());
    }
    let body: serde_json::Value = request.send().await?.json().await?;
    let status = body.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if status != "ok" {
        anyhow::bail!(
            "executor diagnostics {status}: {}",
            body.get("error")
                .or_else(|| body.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
        );
    }
    Ok(body
        .get("output")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

/// One `du` line: megabytes and path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryUsage {
    pub megabytes: u64,
    pub path: String,
}

/// Parse `du -xm -d N …` output (size in MB, tab, path), largest first.
pub fn parse_du_megabytes(output: &str) -> Vec<DirectoryUsage> {
    let mut rows: Vec<DirectoryUsage> = output
        .lines()
        .filter_map(|line| {
            let (size, path) = line.split_once(['\t', ' '])?;
            let megabytes = size.trim().parse::<u64>().ok()?;
            let path = path.trim();
            if path.is_empty() {
                return None;
            }
            Some(DirectoryUsage {
                megabytes,
                path: path.to_string(),
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        b.megabytes
            .cmp(&a.megabytes)
            .then_with(|| a.path.cmp(&b.path))
    });
    rows
}

/// Keep the largest entries, dropping a parent when one of its listed
/// children already explains it (so `/home/ubuntu` and
/// `/home/ubuntu/aegis-runtime` do not both take a slot).
pub fn largest_leafish(rows: &[DirectoryUsage], limit: usize) -> Vec<DirectoryUsage> {
    let mut out: Vec<DirectoryUsage> = Vec::new();
    for row in rows {
        if out.len() >= limit {
            break;
        }
        let prefix = format!("{}/", row.path);
        let has_listed_child = rows.iter().any(|other| {
            other.path != row.path
                && other.path.starts_with(&prefix)
                && other.megabytes * 2 >= row.megabytes
        });
        if has_listed_child {
            continue;
        }
        out.push(row.clone());
    }
    out
}

pub fn format_megabytes(mb: u64) -> String {
    if mb >= 1024 {
        format!("{:.1} GB", mb as f64 / 1024.0)
    } else {
        format!("{mb} MB")
    }
}

// `-m -d 2`, never `-s` (GNU du rejects -s together with --max-depth).
// No `/root`: the executor's protected-path rule rejects it (and it is not
// owner data anyway).
const LARGEST_DIRS_COMMAND: &str = "du -xm -d 2 /home /opt /var /tmp";
const LARGEST_DIRS_TTL: Duration = Duration::from_secs(90 * 60);
static LARGEST_DIRS_CACHE: Mutex<Option<(Instant, Vec<DirectoryUsage>)>> = Mutex::new(None);

/// Largest directories on the host, via the executor, cached for 90 minutes
/// (a disk alert repeats every two hours; one scan per alert is enough).
/// `None` when the executor is unreachable or the scan times out — the alert
/// is then sent without the block, never delayed past `timeout`.
pub async fn largest_directories(limit: usize, timeout: Duration) -> Option<Vec<DirectoryUsage>> {
    let cached = LARGEST_DIRS_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.clone())
        .filter(|(at, _)| at.elapsed() < LARGEST_DIRS_TTL)
        .map(|(_, rows)| rows);
    if let Some(rows) = cached {
        return Some(largest_leafish(&rows, limit));
    }
    let output = match diagnostics(LARGEST_DIRS_COMMAND, timeout).await {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(error = %error, "[Executor] largest-directories scan failed");
            return None;
        }
    };
    let rows = parse_du_megabytes(&output);
    if rows.is_empty() {
        tracing::warn!(
            output_head = %output.chars().take(200).collect::<String>(),
            "[Executor] largest-directories scan returned no du rows"
        );
        return None;
    }
    if let Ok(mut cache) = LARGEST_DIRS_CACHE.lock() {
        *cache = Some((Instant::now(), rows.clone()));
    }
    Some(largest_leafish(&rows, limit))
}

/// The block appended to a disk alert: heading + one line per directory.
pub fn largest_directories_block(rows: &[DirectoryUsage], locale: &str) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::from(crate::i18n::t("alert.heartbeat.disk.largest", locale));
    for row in rows {
        out.push('\n');
        out.push_str(&format!("{} {}", format_megabytes(row.megabytes), row.path));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_du_and_ranks_largest_first() {
        let rows = parse_du_megabytes(
            "120\t/home/ubuntu/x\n34000\t/opt/aegis-runtime\n27000\t/home/ubuntu/aegis-runtime\n",
        );
        assert_eq!(rows[0].path, "/opt/aegis-runtime");
        assert_eq!(rows[0].megabytes, 34000);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn drops_parents_explained_by_a_listed_child() {
        let rows = parse_du_megabytes(
            "40000\t/home/ubuntu\n34000\t/home/ubuntu/aegis-runtime\n34000\t/opt/aegis-runtime\n33000\t/opt/aegis-runtime/build-cache\n2800\t/home/ubuntu/tmp\n",
        );
        let top = largest_leafish(&rows, 3);
        let paths: Vec<&str> = top.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "/home/ubuntu/aegis-runtime",
                "/opt/aegis-runtime/build-cache",
                "/home/ubuntu/tmp"
            ]
        );
    }

    #[test]
    fn formats_block_in_owner_language() {
        let rows = vec![DirectoryUsage {
            megabytes: 34000,
            path: "/opt/aegis-runtime/build-cache".into(),
        }];
        let zh = largest_directories_block(&rows, "zh");
        assert!(zh.starts_with("占用最多的目录："), "{zh}");
        assert!(
            zh.contains("33.2 GB /opt/aegis-runtime/build-cache"),
            "{zh}"
        );
        assert!(largest_directories_block(&rows, "en").starts_with("Largest directories:"));
        assert_eq!(largest_directories_block(&[], "zh"), "");
    }
}
