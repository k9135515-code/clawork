use crate::{
    connectors_google_drive_create_inner, connectors_google_sheets_append_inner,
    connectors_notion_page_create_inner, connectors_notion_search_inner,
    connectors_slack_history_inner, connectors_slack_post_inner, AppState, CommandError,
    GoogleDriveCreateReq, GoogleSheetsAppendReq, NotionPageCreateReq, NotionSearchReq,
    SlackHistoryReq, SlackPostReq,
};
use serde_json::Value;

pub(crate) async fn maybe_call_local_connector_mcp_tool(
    state: &AppState,
    tool_name: &str,
    payload: &Value,
    approval_token: Option<String>,
) -> Result<Option<clawork_core::ToolResult>, CommandError> {
    let normalized = tool_name.to_ascii_lowercase();
    let result = match normalized.as_str() {
        "connector.google.sheets_append" => {
            let spreadsheet_id = payload_required_string(payload, "spreadsheet_id")?;
            let values = payload
                .get("values")
                .cloned()
                .ok_or_else(|| CommandError::validation("payload.values is required"))?;
            let values: Vec<Vec<String>> = serde_json::from_value(values)
                .map_err(|e| CommandError::validation(format!("invalid payload.values: {e}")))?;
            let sheet_name = payload_optional_string(payload, "sheet_name");
            let value_input_option = payload_optional_string(payload, "value_input_option");
            let res = connectors_google_sheets_append_inner(
                state,
                GoogleSheetsAppendReq {
                    spreadsheet_id,
                    sheet_name,
                    values,
                    value_input_option,
                    approval_token,
                },
            )
            .await?;
            serde_json::to_value(res).map_err(|e| CommandError::internal(e.to_string()))?
        }
        "connector.google.drive_create" => {
            let name = payload_required_string(payload, "name")?;
            let content = payload_required_string(payload, "content")?;
            let parent_id = payload_optional_string(payload, "parent_id");
            let mime_type = payload_optional_string(payload, "mime_type");
            let res = connectors_google_drive_create_inner(
                state,
                GoogleDriveCreateReq {
                    name,
                    parent_id,
                    mime_type,
                    content,
                    approval_token,
                },
            )
            .await?;
            serde_json::to_value(res).map_err(|e| CommandError::internal(e.to_string()))?
        }
        "connector.notion.search" => {
            let query = payload_required_string(payload, "query")?;
            let page_size = payload
                .get("page_size")
                .and_then(Value::as_u64)
                .map(|v| v as usize);
            let res = connectors_notion_search_inner(
                state,
                NotionSearchReq {
                    query,
                    page_size,
                    approval_token,
                },
            )
            .await?;
            serde_json::to_value(res).map_err(|e| CommandError::internal(e.to_string()))?
        }
        "connector.notion.page_create" => {
            let title = payload_required_string(payload, "title")?;
            let content = payload_optional_string(payload, "content");
            let parent_page_id = payload_optional_string(payload, "parent_page_id");
            let parent_database_id = payload_optional_string(payload, "parent_database_id");
            let res = connectors_notion_page_create_inner(
                state,
                NotionPageCreateReq {
                    title,
                    content,
                    parent_page_id,
                    parent_database_id,
                    approval_token,
                },
            )
            .await?;
            serde_json::to_value(res).map_err(|e| CommandError::internal(e.to_string()))?
        }
        "connector.slack.post" => {
            let channel = payload_required_string(payload, "channel")?;
            let text = payload_required_string(payload, "text")?;
            let res = connectors_slack_post_inner(
                state,
                SlackPostReq {
                    channel,
                    text,
                    approval_token,
                },
            )
            .await?;
            serde_json::to_value(res).map_err(|e| CommandError::internal(e.to_string()))?
        }
        "connector.slack.history" => {
            let channel = payload_required_string(payload, "channel")?;
            let limit = payload
                .get("limit")
                .and_then(Value::as_u64)
                .map(|v| v as usize);
            let res = connectors_slack_history_inner(
                state,
                SlackHistoryReq {
                    channel,
                    limit,
                    approval_token,
                },
            )
            .await?;
            serde_json::to_value(res).map_err(|e| CommandError::internal(e.to_string()))?
        }
        _ => return Ok(None),
    };
    Ok(Some(clawork_core::ToolResult {
        ok: true,
        payload: result,
        error: None,
    }))
}

fn payload_required_string(payload: &Value, key: &str) -> Result<String, CommandError> {
    let value = payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| CommandError::validation(format!("payload.{key} is required")))?;
    Ok(value.to_string())
}

fn payload_optional_string(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}
