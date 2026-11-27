use std::sync::{Arc, Mutex};

use anyhow::Context;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::time::{Instant, timeout_at};

use super::transport::AgentIo;

const STDERR_TAIL_CAP: usize = 4 * 1024;

pub struct LineRpc {
    stdin: Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
    stdout: Box<dyn tokio::io::AsyncBufRead + Unpin + Send>,
    child: Option<tokio::process::Child>,
    next_id: u64,
    stderr_buf: Arc<Mutex<Vec<u8>>>,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
}

impl LineRpc {
    pub fn from_io(io: AgentIo) -> Self {
        let (stderr_buf, stderr_task) = spawn_stderr_drain(io.stderr);
        Self {
            stdin: io.stdin,
            stdout: io.stdout,
            child: io.child,
            next_id: 1,
            stderr_buf,
            stderr_task,
        }
    }

    pub fn kill_child(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
    }

    pub fn stderr_tail(&self) -> Option<String> {
        let guard = self
            .stderr_buf
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if guard.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(&guard).into_owned())
        }
    }

    pub fn wrap_err(&self, err: anyhow::Error) -> anyhow::Error {
        match self.stderr_tail() {
            Some(tail) => anyhow::anyhow!("{err} — stderr: {tail}"),
            None => err,
        }
    }

    pub async fn abort(&mut self) {
        let _ = self.stdin.shutdown().await;
        self.kill_child();
        if let Some(handle) = self.stderr_task.take() {
            let _ = tokio::time::timeout(std::time::Duration::from_millis(100), handle).await;
        }
    }

    pub async fn notify(&mut self, method: &str, params: Option<Value>) -> anyhow::Result<()> {
        let mut message = json!({
            "jsonrpc": "2.0",
            "method": method,
        });
        if let Some(params) = params
            && let Some(obj) = message.as_object_mut()
        {
            obj.insert("params".to_string(), params);
        }
        self.write_value(&message)
            .await
            .with_context(|| format!("write JSON-RPC notification {method}"))
    }

    pub async fn request(
        &mut self,
        method: &str,
        params: Value,
        deadline: Instant,
        on_notification: &mut (dyn FnMut(&str, &Value) + Send),
        on_agent_request: &mut (dyn FnMut(&str, &Value) -> Value + Send),
    ) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_value(&message)
            .await
            .with_context(|| format!("write JSON-RPC request {method}"))?;

        loop {
            match self.read_event(deadline, method).await? {
                LineEvent::Eof => {
                    anyhow::bail!("agent closed stdout before responding to {method}");
                }
                LineEvent::Skip => {}
                LineEvent::Notification { method, params } => {
                    on_notification(&method, &params);
                }
                LineEvent::AgentRequest { method, id, params } => {
                    self.reply_agent_request(&id, &method, &params, on_agent_request)
                        .await?;
                }
                LineEvent::Response(message) => {
                    if !id_matches(message.get("id"), id) {
                        continue;
                    }
                    if let Some(error) = message.get("error") {
                        anyhow::bail!("JSON-RPC error on {method}: {error}");
                    }
                    return Ok(message.get("result").cloned().unwrap_or(Value::Null));
                }
            }
        }
    }

    pub async fn wait_until(
        &mut self,
        deadline: Instant,
        wait_for: &str,
        on_notification: &mut (dyn FnMut(&str, &Value) -> bool + Send),
        on_agent_request: &mut (dyn FnMut(&str, &Value) -> Value + Send),
    ) -> anyhow::Result<()> {
        loop {
            match self.read_event(deadline, wait_for).await? {
                LineEvent::Eof => {
                    anyhow::bail!("agent closed stdout before responding to {wait_for}");
                }
                LineEvent::Skip | LineEvent::Response(_) => {}
                LineEvent::Notification { method, params } => {
                    if on_notification(&method, &params) {
                        return Ok(());
                    }
                }
                LineEvent::AgentRequest { method, id, params } => {
                    self.reply_agent_request(&id, &method, &params, on_agent_request)
                        .await?;
                }
            }
        }
    }

    async fn reply_agent_request(
        &mut self,
        id: &Value,
        method: &str,
        params: &Value,
        on_agent_request: &mut (dyn FnMut(&str, &Value) -> Value + Send),
    ) -> anyhow::Result<()> {
        let result = on_agent_request(method, params);
        let reply = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        });
        self.write_value(&reply)
            .await
            .with_context(|| format!("write JSON-RPC reply to agent request {method}"))
    }

    async fn write_value(&mut self, value: &Value) -> anyhow::Result<()> {
        let mut line = serde_json::to_vec(value).context("serialize JSON-RPC message")?;
        line.push(b'\n');
        self.stdin
            .write_all(&line)
            .await
            .context("write JSON-RPC line")?;
        self.stdin.flush().await.context("flush JSON-RPC stdin")?;
        Ok(())
    }

    async fn read_event(&mut self, deadline: Instant, wait_for: &str) -> anyhow::Result<LineEvent> {
        let mut buf = Vec::new();
        match timeout_at(deadline, self.stdout.read_until(b'\n', &mut buf)).await {
            Err(_) => anyhow::bail!("timeout waiting for {wait_for}"),
            Ok(Err(err)) => Err(err).context("read agent stdout")?,
            Ok(Ok(0)) => {
                if buf.is_empty() {
                    Ok(LineEvent::Eof)
                } else {
                    Ok(classify_line(&buf))
                }
            }
            Ok(Ok(_)) => Ok(classify_line(&buf)),
        }
    }
}

enum LineEvent {
    Notification {
        method: String,
        params: Value,
    },
    AgentRequest {
        method: String,
        id: Value,
        params: Value,
    },
    Response(Value),
    Skip,
    Eof,
}

fn classify_line(buf: &[u8]) -> LineEvent {
    let line = String::from_utf8_lossy(buf);
    let trimmed = line.trim();
    if trimmed.is_empty() {
        tracing::debug!("skipping empty JSON-RPC line");
        return LineEvent::Skip;
    }
    let parsed = match serde_json::from_str::<Value>(trimmed) {
        Ok(value) => value,
        Err(err) => {
            tracing::debug!(line = trimmed, error = %err, "skipping unparsable JSON-RPC line");
            return LineEvent::Skip;
        }
    };
    let method = parsed
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);
    let id = parsed.get("id").cloned().filter(|id| !id.is_null());
    let params = parsed.get("params").cloned().unwrap_or(Value::Null);
    match (method, id) {
        (Some(method), None) => LineEvent::Notification { method, params },
        (Some(method), Some(id)) => LineEvent::AgentRequest { method, id, params },
        (None, Some(_)) => LineEvent::Response(parsed),
        (None, None) => {
            tracing::debug!(
                line = trimmed,
                "skipping JSON-RPC line without method or id"
            );
            LineEvent::Skip
        }
    }
}

fn spawn_stderr_drain(
    stderr: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>,
) -> (Arc<Mutex<Vec<u8>>>, Option<tokio::task::JoinHandle<()>>) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let Some(mut stderr) = stderr else {
        return (buf, None);
    };
    let buf_task = Arc::clone(&buf);
    let handle = tokio::spawn(async move {
        let mut chunk = [0_u8; 256];
        loop {
            match stderr.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut guard = buf_task.lock().unwrap_or_else(|err| err.into_inner());
                    guard.extend_from_slice(&chunk[..n]);
                    if guard.len() > STDERR_TAIL_CAP {
                        let overflow = guard.len() - STDERR_TAIL_CAP;
                        guard.drain(..overflow);
                    }
                }
            }
        }
    });
    (buf, Some(handle))
}

fn id_matches(actual: Option<&Value>, expected: u64) -> bool {
    let Some(actual) = actual else {
        return false;
    };
    if actual.as_u64() == Some(expected) {
        return true;
    }
    if actual.as_i64() == Some(i64::try_from(expected).unwrap_or(i64::MAX)) {
        return true;
    }
    actual.as_str() == Some(expected.to_string().as_str())
}
