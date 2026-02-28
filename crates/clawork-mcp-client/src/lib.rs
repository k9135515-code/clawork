use anyhow::Context;
use async_trait::async_trait;
use clawork_core::{ToolArgs, ToolCaller, ToolResult};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

struct McpSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    request_id: u64,
}

#[derive(Clone)]
pub struct StdioMcpClient {
    command: String,
    args: Vec<String>,
    timeout: Duration,
    session: Arc<Mutex<Option<McpSession>>>,
}

impl StdioMcpClient {
    pub fn new(command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            command: command.into(),
            args,
            timeout: Duration::from_secs(15),
            session: Arc::new(Mutex::new(None)),
        }
    }

    async fn spawn_session(&self) -> anyhow::Result<McpSession> {
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn MCP server: {}", self.command))?;

        let stdin = child.stdin.take().context("failed to capture MCP stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("failed to capture MCP stdout")?;

        Ok(McpSession {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            request_id: 1,
        })
    }

    async fn ensure_session<'a>(
        &'a self,
        guard: &'a mut Option<McpSession>,
    ) -> anyhow::Result<&'a mut McpSession> {
        let mut needs_spawn = guard.is_none();
        if let Some(session) = guard.as_mut() {
            if let Some(_status) = session.child.try_wait()? {
                needs_spawn = true;
            }
        }

        if needs_spawn {
            *guard = Some(self.spawn_session().await?);
        }

        guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("failed to initialize MCP session"))
    }

    async fn rpc(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let mut session_guard = self.session.lock().await;

        for attempt in 0..2 {
            let session = self.ensure_session(&mut session_guard).await?;

            let id = session.request_id;
            session.request_id = session.request_id.saturating_add(1);

            let request = json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params
            })
            .to_string();

            let write_result = timeout(self.timeout, async {
                session.stdin.write_all(request.as_bytes()).await?;
                session.stdin.write_all(b"\n").await?;
                session.stdin.flush().await?;
                Ok::<(), std::io::Error>(())
            })
            .await;

            if write_result.is_err() {
                *session_guard = None;
                if attempt == 1 {
                    return Err(anyhow::anyhow!("mcp write timed out"));
                }
                continue;
            }

            let mut line = String::new();
            let read_result = timeout(self.timeout, session.stdout.read_line(&mut line)).await;
            match read_result {
                Ok(Ok(0)) => {
                    *session_guard = None;
                    if attempt == 1 {
                        return Err(anyhow::anyhow!("mcp server closed stdout"));
                    }
                }
                Ok(Ok(_)) => {
                    let parsed = serde_json::from_str::<Value>(line.trim()).unwrap_or_else(|_| {
                        json!({
                            "result": {
                                "status": "unparsed",
                                "raw": line.trim()
                            }
                        })
                    });
                    return Ok(parsed);
                }
                Ok(Err(err)) => {
                    *session_guard = None;
                    if attempt == 1 {
                        return Err(anyhow::anyhow!("mcp read error: {err}"));
                    }
                }
                Err(_) => {
                    *session_guard = None;
                    if attempt == 1 {
                        return Err(anyhow::anyhow!("mcp read timed out"));
                    }
                }
            }
        }

        Err(anyhow::anyhow!("mcp rpc failed after retry"))
    }

    pub async fn initialize(&self) -> anyhow::Result<Value> {
        self.rpc(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": true
                },
                "clientInfo": {
                    "name": "clawork",
                    "version": "0.1.0"
                }
            }),
        )
        .await
    }

    pub async fn list_tools(&self) -> anyhow::Result<Value> {
        self.rpc("tools/list", json!({})).await
    }
}

#[async_trait]
impl ToolCaller for StdioMcpClient {
    async fn call(&self, tool_name: &str, args: ToolArgs) -> anyhow::Result<ToolResult> {
        let _ = self.initialize().await;
        let parsed = self
            .rpc(
                "tools/call",
                json!({
                    "name": tool_name,
                    "arguments": args.payload
                }),
            )
            .await?;

        let payload = parsed
            .get("result")
            .cloned()
            .unwrap_or_else(|| json!({ "status": "no_result" }));
        let error = parsed
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .map(ToString::to_string);

        Ok(ToolResult {
            ok: error.is_none(),
            payload,
            error,
        })
    }
}
