use crate::{AppState, CommandError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
struct RemoteMcpCallReq {
    tool_name: String,
    payload: Value,
}

#[derive(Debug, Deserialize)]
struct RemoteMcpCallRes {
    ok: bool,
    payload: Value,
    error: Option<String>,
}

pub(crate) fn mcp_target_for_route(route: &str, tool_name: &str) -> Result<String, CommandError> {
    match route {
        "local" => Ok(format!("mcp://local/{tool_name}")),
        "remote" => std::env::var("CLAWORK_REMOTE_MCP_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                CommandError::not_configured("CLAWORK_REMOTE_MCP_URL is required for route=remote")
            }),
        other => Err(CommandError::validation(format!(
            "unsupported mcp route: {other} (use local or remote)"
        ))),
    }
}

pub(crate) async fn call_remote_mcp_tool(
    state: &AppState,
    tool_name: &str,
    payload: Value,
    endpoint: String,
) -> Result<clawork_core::ToolResult, CommandError> {
    let request = RemoteMcpCallReq {
        tool_name: tool_name.to_string(),
        payload,
    };
    let resp = state
        .http
        .post(endpoint)
        .json(&request)
        .send()
        .await
        .map_err(|e| CommandError::internal(format!("remote mcp request failed: {e}")))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| CommandError::internal(format!("remote mcp response parse failed: {e}")))?;
    if !status.is_success() {
        return Err(CommandError::internal(format!(
            "remote mcp failed ({}): {}",
            status.as_u16(),
            body
        )));
    }

    let mut parsed: RemoteMcpCallRes = serde_json::from_value(body.clone()).map_err(|e| {
        CommandError::validation(format!(
            "remote mcp response schema mismatch: expected {{ok:boolean,payload:any,error?:string}}; parse_error={e}; body={body}"
        ))
    })?;
    if !parsed.ok && parsed.error.is_none() {
        parsed.error = Some("remote_mcp_error".to_string());
    }

    Ok(clawork_core::ToolResult {
        ok: parsed.ok,
        payload: parsed.payload,
        error: parsed.error,
    })
}
