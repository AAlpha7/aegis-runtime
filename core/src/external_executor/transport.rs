use std::process::Stdio;

use anyhow::Context;
use tokio::io::BufReader;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: std::path::PathBuf,
    pub env: Vec<(String, String)>,
}

pub struct AgentIo {
    pub stdin: Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
    pub stdout: Box<dyn tokio::io::AsyncBufRead + Unpin + Send>,
    /// `Some` for real processes; `None` for in-memory fakes.
    pub child: Option<tokio::process::Child>,
    /// Piped child stderr; fakes may pass `None`.
    pub stderr: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>,
}

pub trait AgentTransport: Send + Sync {
    fn spawn(&self, command: &AgentCommand) -> anyhow::Result<AgentIo>;
}

/// Real child process with a clean allowlisted environment (API keys excluded),
/// piped stdio, kill-on-drop cleanup, and an explicit working directory.
pub struct ProcessTransport;

const CHILD_ENV_EXACT: &[&str] = &[
    "PATH",
    "HOME",
    "LANG",
    "LC_ALL",
    "TERM",
    "XDG_RUNTIME_DIR",
    "TMPDIR",
];

const CHILD_ENV_PREFIXES: &[&str] = &["GROK_", "CODEX_", "XAI_"];

fn env_key_allowed(key: &str) -> bool {
    if key.ends_with("API_KEY") {
        return false;
    }
    CHILD_ENV_EXACT.contains(&key)
        || CHILD_ENV_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(prefix))
}

fn push_or_replace(env: &mut Vec<(String, String)>, key: String, value: String) {
    if let Some(existing) = env.iter_mut().find(|(k, _)| *k == key) {
        existing.1 = value;
    } else {
        env.push((key, value));
    }
}

/// Build the child environment from a host snapshot plus command extras.
///
/// Aegis Runtime secrets (`AEGIS_*`, `TELEGRAM_*`, `OPENROUTER_*`, `ADMIN_API_TOKEN`,
/// `HERCULES_*`, …) are dropped unless they are explicitly present in `extra`.
pub(crate) fn child_env(
    host: &[(String, String)],
    extra: &[(String, String)],
) -> Vec<(String, String)> {
    let mut env = Vec::new();
    for (key, value) in host {
        if env_key_allowed(key) {
            push_or_replace(&mut env, key.clone(), value.clone());
        }
    }
    for (key, value) in extra {
        push_or_replace(&mut env, key.clone(), value.clone());
    }
    env
}

impl AgentTransport for ProcessTransport {
    fn spawn(&self, command: &AgentCommand) -> anyhow::Result<AgentIo> {
        let host: Vec<(String, String)> = std::env::vars().collect();
        let env = child_env(&host, &command.env);
        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args)
            .current_dir(&command.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env_clear()
            .envs(env);
        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn agent binary '{}'", command.program))?;
        let stdin = child.stdin.take().context("agent stdin unavailable")?;
        let stdout = child.stdout.take().context("agent stdout unavailable")?;
        let stderr = child.stderr.take().context("agent stderr unavailable")?;
        Ok(AgentIo {
            stdin: Box::new(stdin),
            stdout: Box::new(BufReader::new(stdout)),
            child: Some(child),
            stderr: Some(Box::new(stderr)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::child_env;

    #[test]
    fn child_env_allow_list_keeps_runtime_and_executor_keys() {
        let host = [
            ("PATH".to_string(), "/bin".to_string()),
            ("HOME".to_string(), "/home/u".to_string()),
            ("LC_ALL".to_string(), "C.UTF-8".to_string()),
            ("TERM".to_string(), "xterm".to_string()),
            ("XDG_RUNTIME_DIR".to_string(), "/run/user/1".to_string()),
            ("TMPDIR".to_string(), "/tmp".to_string()),
            ("AEGIS_EXECUTOR_GROK_BIN".to_string(), "drop".to_string()),
            ("TELEGRAM_BOT_TOKEN".to_string(), "drop".to_string()),
            ("HERCULES_URL".to_string(), "drop".to_string()),
            ("GROK_MODEL".to_string(), "keep".to_string()),
            ("CODEX_HOME".to_string(), "/opt/codex".to_string()),
            ("XAI_API_KEY".to_string(), "keep-xai".to_string()),
        ];
        let extra = [("CUSTOM".to_string(), "from-command".to_string())];
        let env = child_env(&host, &extra);
        let get = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        assert_eq!(get("PATH"), Some("/bin"));
        assert_eq!(get("GROK_MODEL"), Some("keep"));
        assert_eq!(get("CODEX_HOME"), Some("/opt/codex"));
        assert_eq!(get("XAI_API_KEY"), None);
        assert_eq!(get("CUSTOM"), Some("from-command"));
        assert_eq!(get("AEGIS_EXECUTOR_GROK_BIN"), None);
        assert_eq!(get("TELEGRAM_BOT_TOKEN"), None);
        assert_eq!(get("HERCULES_URL"), None);
    }
}
