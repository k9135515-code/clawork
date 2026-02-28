use crate::local_auth::{
    issue_local_api_token, require_auth, revoke_local_api_token, AuthTokenIssueReq,
    AuthTokenResult, AuthTokenRevokeReq, AuthTokenRotateReq,
};
use crate::{
    append_audit, authorize_action, browser_navigate_inner, call_mcp_tool_inner, config_set_inner,
    config_show_inner, daemon_restart_inner, daemon_start_inner, daemon_stop_inner,
    ensure_action_still_authorized, fs_operate_inner, get_audit_events_inner,
    get_daily_briefing_inner, list_inbound_messages_inner, list_proactive_suggestions_inner,
    mail_inbox_unreplied_inner, memory_recent_inner, memory_search_inner, memory_store_inner,
    office_create_excel_inner, office_upload_graph_file_inner, policy_list_domain_inner,
    policy_set_domain_inner, send_message_inner, status_inner, task_run_inner, AppState,
    ApproveReq, BrowserNavigateApiReq, CommandError, ConfigSetReq, CreateSkillReq, FsOperateApiReq,
    InboundQuery, LimitQuery, MailInboxItem, MailInboxQuery, McpCallReq, MemorySearchReq,
    MemoryStoreReq, OfficeExcelApiReq, OfficeUploadApiReq, RunSkillReq, SendMessageReq, TaskRunReq,
};
use axum::extract::{Query, State as AxumState};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use clawork_core::{
    ActionKind, ActionRequest, AuditEvent, BrowserRunRequest, BrowserRunResult, DomainPolicy,
    FsOperationKind, FsOperationRequest, FsOperationResult, InboundMessage, OfficeUploadResult,
    SkillRequest, SkillResponse, SkillRuntime, TaskInfo,
};
use clawork_memory::{MemoryHit, MemoryRecord};
use clawork_skills::SkillManifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

pub(crate) async fn api_get_status(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<clawork_core::AppStatus>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(status_inner(&state).await))
}

pub(crate) async fn api_get_tasks(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<TaskInfo>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(state.daemon.list_tasks()))
}

pub(crate) async fn api_list_skills(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<SkillManifest>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(state.registry.list()))
}

pub(crate) async fn api_approve_action(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ApproveReq>,
) -> Result<Json<bool>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(state.permissions.approve_token(&body.token)))
}

pub(crate) async fn api_auth_token_issue(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<AuthTokenIssueReq>,
) -> Result<Json<AuthTokenResult>, CommandError> {
    require_auth(&state, &headers)?;
    let ttl = body.ttl_seconds.or(Some(3600));
    Ok(Json(issue_local_api_token(&state, ttl, false)?))
}

pub(crate) async fn api_auth_token_revoke(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<AuthTokenRevokeReq>,
) -> Result<Json<bool>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(revoke_local_api_token(&state, body.token.trim())))
}

pub(crate) async fn api_auth_token_rotate(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<AuthTokenRotateReq>,
) -> Result<Json<AuthTokenResult>, CommandError> {
    require_auth(&state, &headers)?;
    let ttl = body.ttl_seconds;
    let result = issue_local_api_token(&state, ttl, true)?;
    let mut tokens = state.local_api_tokens.write();
    tokens.retain(|k, _| k == &result.token);
    Ok(Json(result))
}

pub(crate) async fn api_run_skill(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<RunSkillReq>,
) -> Result<Json<SkillResponse>, CommandError> {
    require_auth(&state, &headers)?;
    let action = ActionRequest {
        kind: ActionKind::SkillExecute,
        target: Some(body.skill_id.clone()),
        params: body.input.clone(),
        trace_id: Uuid::new_v4(),
    };
    let ctx = authorize_action(&state, &action, body.approval_token).await?;
    ensure_action_still_authorized(&state, &action, &ctx).await?;
    let response = state
        .runtime
        .execute(
            &body.skill_id,
            SkillRequest {
                abi_version: "v1".into(),
                skill_id: body.skill_id.clone(),
                input: body.input,
                trace_id: action.trace_id,
            },
        )
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;

    append_audit(
        &state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: "allowed".into(),
            reason: None,
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(Json(response))
}

pub(crate) async fn api_create_skill(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateSkillReq>,
) -> Result<Json<SkillManifest>, CommandError> {
    require_auth(&state, &headers)?;
    let action = ActionRequest {
        kind: ActionKind::SkillInstall,
        target: Some(body.skill_id.clone()),
        params: serde_json::json!({ "name": body.name.clone(), "description": body.description.clone() }),
        trace_id: Uuid::new_v4(),
    };
    let ctx = authorize_action(&state, &action, body.approval_token).await?;
    ensure_action_still_authorized(&state, &action, &ctx).await?;
    let manifest = state
        .registry
        .create_skill_template(&body.skill_id, &body.name, body.description)
        .map_err(|e| CommandError::validation(e.to_string()))?;
    append_audit(
        &state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: "allowed".into(),
            reason: None,
            trace_id: action.trace_id,
        },
    )
    .await;
    Ok(Json(manifest))
}

pub(crate) async fn api_send_message(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SendMessageReq>,
) -> Result<Json<clawork_core::SendResult>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        send_message_inner(
            &state,
            body.adapter,
            body.to,
            body.content,
            body.approval_token,
        )
        .await?,
    ))
}

pub(crate) async fn api_fs_operate(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<FsOperateApiReq>,
) -> Result<Json<FsOperationResult>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        fs_operate_inner(&state, body.op, body.approval_token).await?,
    ))
}

pub(crate) async fn api_mcp_call(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<McpCallReq>,
) -> Result<Json<clawork_core::ToolResult>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        call_mcp_tool_inner(
            &state,
            body.tool_name,
            body.payload,
            body.route,
            body.approval_token,
        )
        .await?,
    ))
}

pub(crate) async fn api_browser_navigate(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<BrowserNavigateApiReq>,
) -> Result<Json<BrowserRunResult>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        browser_navigate_inner(&state, body.req, body.approval_token).await?,
    ))
}

pub(crate) async fn api_office_excel(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<OfficeExcelApiReq>,
) -> Result<Json<String>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        office_create_excel_inner(&state, body.req, body.approval_token).await?,
    ))
}

pub(crate) async fn api_office_upload(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<OfficeUploadApiReq>,
) -> Result<Json<OfficeUploadResult>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        office_upload_graph_file_inner(&state, body.req, body.approval_token).await?,
    ))
}

pub(crate) async fn api_inbound_messages(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<InboundQuery>,
) -> Result<Json<Vec<InboundMessage>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        list_inbound_messages_inner(&state, query.limit, query.adapter).await?,
    ))
}

pub(crate) async fn api_mail_inbox_unreplied(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<MailInboxQuery>,
) -> Result<Json<Vec<MailInboxItem>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        mail_inbox_unreplied_inner(&state, query.limit, query.approval_token).await?,
    ))
}

pub(crate) async fn api_memory_store(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<MemoryStoreReq>,
) -> Result<Json<String>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        memory_store_inner(&state, body.text, body.embedding, body.approval_token).await?,
    ))
}

pub(crate) async fn api_memory_recent(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LimitQuery>,
) -> Result<Json<Vec<MemoryRecord>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(memory_recent_inner(&state, query.limit).await?))
}

pub(crate) async fn api_memory_search(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<MemorySearchReq>,
) -> Result<Json<Vec<MemoryHit>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(
        memory_search_inner(&state, body.query, body.limit).await?,
    ))
}

pub(crate) async fn api_daemon_start(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<crate::GenericOk>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(daemon_start_inner(&state).await?))
}

pub(crate) async fn api_daemon_stop(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<crate::GenericOk>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(daemon_stop_inner(&state).await?))
}

pub(crate) async fn api_daemon_restart(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<crate::GenericOk>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(daemon_restart_inner(&state).await?))
}

pub(crate) async fn api_task_run(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TaskRunReq>,
) -> Result<Json<crate::GenericOk>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(task_run_inner(&state, body.task_id).await?))
}

pub(crate) async fn api_logs(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LimitQuery>,
) -> Result<Json<Vec<AuditEvent>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(get_audit_events_inner(&state, query.limit).await?))
}

pub(crate) async fn api_config_show(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(config_show_inner(&state).await?))
}

pub(crate) async fn api_config_set(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ConfigSetReq>,
) -> Result<Json<crate::GenericOk>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(config_set_inner(&state, body.key, body.value).await?))
}

pub(crate) async fn api_policy_set_domain(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<crate::DomainPolicySetReq>,
) -> Result<Json<DomainPolicy>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(policy_set_domain_inner(&state, body).await?))
}

pub(crate) async fn api_policy_list_domain(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<DomainPolicy>>, CommandError> {
    require_auth(&state, &headers)?;
    Ok(Json(policy_list_domain_inner(&state).await?))
}

#[derive(Debug, Deserialize)]
pub(crate) struct NlExecuteReq {
    instruction: String,
    dry_run: Option<bool>,
    continue_on_error: Option<bool>,
    approval_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NlPlannedStep {
    action: String,
    #[serde(default)]
    params: Value,
    rationale: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct NlStepOutcome {
    index: usize,
    action: String,
    ok: bool,
    result: Option<Value>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct NlExecuteResult {
    instruction: String,
    plan_source: String,
    model: Option<String>,
    dry_run: bool,
    steps: Vec<NlPlannedStep>,
    outcomes: Vec<NlStepOutcome>,
}

pub(crate) async fn api_nl_execute(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NlExecuteReq>,
) -> Result<Json<NlExecuteResult>, CommandError> {
    require_auth(&state, &headers)?;
    if body.instruction.trim().is_empty() {
        return Err(CommandError::validation("instruction must not be empty"));
    }

    let (steps, plan_source, model) = match build_llm_plan(&state, &body.instruction).await {
        Ok(Some((steps, model))) => (steps, "llm".to_string(), Some(model)),
        _ => (build_rule_plan(&body.instruction), "rule".to_string(), None),
    };

    if steps.is_empty() {
        return Err(CommandError::validation(
            "could not derive executable steps from instruction",
        ));
    }

    let dry_run = body.dry_run.unwrap_or(false);
    let continue_on_error = body.continue_on_error.unwrap_or(false);
    let mut outcomes = Vec::<NlStepOutcome>::new();

    if !dry_run {
        for (idx, step) in steps.iter().enumerate() {
            match execute_planned_step(&state, step, body.approval_token.clone()).await {
                Ok(result) => outcomes.push(NlStepOutcome {
                    index: idx,
                    action: step.action.clone(),
                    ok: true,
                    result: Some(result),
                    error: None,
                }),
                Err(err) => {
                    outcomes.push(NlStepOutcome {
                        index: idx,
                        action: step.action.clone(),
                        ok: false,
                        result: None,
                        error: Some(err.message.clone()),
                    });
                    if !continue_on_error {
                        return Err(err);
                    }
                }
            }
        }
    }

    Ok(Json(NlExecuteResult {
        instruction: body.instruction,
        plan_source,
        model,
        dry_run,
        steps,
        outcomes,
    }))
}

async fn execute_planned_step(
    state: &AppState,
    step: &NlPlannedStep,
    approval_token: Option<String>,
) -> Result<Value, CommandError> {
    let params = &step.params;
    match step.action.as_str() {
        "status" => serde_json::to_value(status_inner(state).await)
            .map_err(|e| CommandError::internal(format!("serialize status: {e}"))),
        "briefing" => {
            let briefing = get_daily_briefing_inner(state).await?;
            serde_json::to_value(briefing)
                .map_err(|e| CommandError::internal(format!("serialize briefing: {e}")))
        }
        "suggestions" => {
            let limit = param_i64(params, "limit").map(|v| v.max(1) as usize);
            let list = list_proactive_suggestions_inner(state, limit).await?;
            serde_json::to_value(list)
                .map_err(|e| CommandError::internal(format!("serialize suggestions: {e}")))
        }
        "daemon_start" => {
            let v = daemon_start_inner(state).await?;
            serde_json::to_value(v).map_err(|e| CommandError::internal(e.to_string()))
        }
        "daemon_stop" => {
            let v = daemon_stop_inner(state).await?;
            serde_json::to_value(v).map_err(|e| CommandError::internal(e.to_string()))
        }
        "daemon_restart" => {
            let v = daemon_restart_inner(state).await?;
            serde_json::to_value(v).map_err(|e| CommandError::internal(e.to_string()))
        }
        "task_run" => {
            let task_id = param_str(params, "task_id")?;
            let v = task_run_inner(state, task_id.to_string()).await?;
            serde_json::to_value(v).map_err(|e| CommandError::internal(e.to_string()))
        }
        "memory_store" => {
            let text = param_str(params, "text")?;
            let id = memory_store_inner(state, text.to_string(), None, approval_token).await?;
            Ok(serde_json::json!({ "id": id }))
        }
        "memory_search" => {
            let query = param_str(params, "query")?;
            let limit = param_i64(params, "limit");
            let hits = memory_search_inner(state, query.to_string(), limit).await?;
            serde_json::to_value(hits).map_err(|e| CommandError::internal(e.to_string()))
        }
        "fs_read" => {
            let path = param_str(params, "path")?;
            let req = FsOperationRequest {
                kind: FsOperationKind::ReadFile,
                path: path.to_string(),
                content: None,
            };
            let res = fs_operate_inner(state, req, approval_token).await?;
            serde_json::to_value(res).map_err(|e| CommandError::internal(e.to_string()))
        }
        "fs_write" => {
            let path = param_str(params, "path")?;
            let content = param_str(params, "content")?;
            let req = FsOperationRequest {
                kind: FsOperationKind::WriteFile,
                path: path.to_string(),
                content: Some(content.to_string()),
            };
            let res = fs_operate_inner(state, req, approval_token).await?;
            serde_json::to_value(res).map_err(|e| CommandError::internal(e.to_string()))
        }
        "message_send" => {
            let adapter = param_str(params, "adapter")?;
            let to = param_str(params, "to")?;
            let content = param_str(params, "content")?;
            let res = send_message_inner(
                state,
                adapter.to_string(),
                to.to_string(),
                content.to_string(),
                approval_token,
            )
            .await?;
            serde_json::to_value(res).map_err(|e| CommandError::internal(e.to_string()))
        }
        "browser_navigate" => {
            let url = param_str(params, "url")?;
            let timeout_seconds = param_i64(params, "timeout_seconds")
                .map(|v| v.max(1) as u64)
                .unwrap_or(30);
            let allow_domains = params
                .get("allow_domains")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let headed = params
                .get("headed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let req = BrowserRunRequest {
                url: url.to_string(),
                allow_domains,
                headed,
                timeout_seconds,
            };
            let res = browser_navigate_inner(state, req, approval_token).await?;
            serde_json::to_value(res).map_err(|e| CommandError::internal(e.to_string()))
        }
        "mcp_call" => {
            let tool_name = param_str(params, "tool_name")?;
            let payload = params
                .get("payload")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let route = params
                .get("route")
                .and_then(Value::as_str)
                .map(str::to_string);
            let res =
                call_mcp_tool_inner(state, tool_name.to_string(), payload, route, approval_token)
                    .await?;
            serde_json::to_value(res).map_err(|e| CommandError::internal(e.to_string()))
        }
        other => Err(CommandError::validation(format!(
            "unsupported action '{other}'"
        ))),
    }
}

fn param_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, CommandError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| CommandError::validation(format!("missing '{key}'")))
}

fn param_i64(params: &Value, key: &str) -> Option<i64> {
    params.get(key).and_then(Value::as_i64)
}

fn build_rule_plan(instruction: &str) -> Vec<NlPlannedStep> {
    let text = instruction.trim();
    let lower = text.to_ascii_lowercase();

    if lower.contains("status") || lower.contains("状態") {
        return vec![NlPlannedStep {
            action: "status".into(),
            params: serde_json::json!({}),
            rationale: Some("status keyword matched".into()),
        }];
    }
    if lower.contains("briefing") || lower.contains("ブリーフィング") || lower.contains("要約")
    {
        return vec![NlPlannedStep {
            action: "briefing".into(),
            params: serde_json::json!({}),
            rationale: Some("briefing/summary keyword matched".into()),
        }];
    }
    if let Some(rest) = text.strip_prefix("memory search ") {
        return vec![NlPlannedStep {
            action: "memory_search".into(),
            params: serde_json::json!({ "query": rest.trim(), "limit": 5 }),
            rationale: Some("memory search command matched".into()),
        }];
    }
    if let Some(rest) = text.strip_prefix("memory store ") {
        return vec![NlPlannedStep {
            action: "memory_store".into(),
            params: serde_json::json!({ "text": rest.trim() }),
            rationale: Some("memory store command matched".into()),
        }];
    }
    if let Some(rest) = text.strip_prefix("fs read ") {
        return vec![NlPlannedStep {
            action: "fs_read".into(),
            params: serde_json::json!({ "path": rest.trim() }),
            rationale: Some("fs read command matched".into()),
        }];
    }
    if let Some(rest) = text.strip_prefix("fs write ") {
        if let Some((path, content)) = rest.split_once("::") {
            return vec![NlPlannedStep {
                action: "fs_write".into(),
                params: serde_json::json!({ "path": path.trim(), "content": content.trim() }),
                rationale: Some("fs write command matched".into()),
            }];
        }
    }
    if let Some(rest) = text.strip_prefix("send ") {
        if let Some((head, content)) = rest.split_once("::") {
            let parts = head.split_whitespace().collect::<Vec<_>>();
            if parts.len() >= 2 {
                return vec![NlPlannedStep {
                    action: "message_send".into(),
                    params: serde_json::json!({
                        "adapter": parts[0],
                        "to": parts[1],
                        "content": content.trim()
                    }),
                    rationale: Some("send command matched".into()),
                }];
            }
        }
    }
    if let Some(url) = text.strip_prefix("open ") {
        return vec![NlPlannedStep {
            action: "browser_navigate".into(),
            params: serde_json::json!({ "url": url.trim(), "timeout_seconds": 30 }),
            rationale: Some("open command matched".into()),
        }];
    }

    vec![
        NlPlannedStep {
            action: "briefing".into(),
            params: serde_json::json!({}),
            rationale: Some("fallback step 1".into()),
        },
        NlPlannedStep {
            action: "suggestions".into(),
            params: serde_json::json!({ "limit": 5 }),
            rationale: Some("fallback step 2".into()),
        },
    ]
}

async fn build_llm_plan(
    state: &AppState,
    instruction: &str,
) -> Result<Option<(Vec<NlPlannedStep>, String)>, CommandError> {
    let api_key = match std::env::var("CLAWORK_OPENAI_API_KEY") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => return Ok(None),
    };
    let model =
        std::env::var("CLAWORK_OPENAI_CHAT_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    let system = "You are a planner for a local desktop agent. Return only JSON: {\"steps\":[{\"action\":\"status|briefing|suggestions|memory_store|memory_search|fs_read|fs_write|message_send|browser_navigate|mcp_call|daemon_start|daemon_stop|daemon_restart|task_run\",\"params\":{},\"rationale\":\"...\"}]}. Use fs write format with params.path and params.content. For message_send include adapter,to,content.";
    let payload = serde_json::json!({
        "model": model,
        "temperature": 0,
        "messages": [
            {"role":"system","content": system},
            {"role":"user","content": instruction}
        ]
    });

    let resp = state
        .http
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| CommandError::internal(format!("llm planner request failed: {e}")))?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| CommandError::internal(format!("llm planner response parse failed: {e}")))?;
    let content = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("message"))
        .and_then(|v| v.get("content"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if content.is_empty() {
        return Ok(None);
    }

    let json_text = strip_markdown_fence(content);
    let parsed: Value = match serde_json::from_str(json_text) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let Some(steps_value) = parsed.get("steps").and_then(Value::as_array) else {
        return Ok(None);
    };
    let mut steps = Vec::<NlPlannedStep>::new();
    for v in steps_value {
        if let Ok(step) = serde_json::from_value::<NlPlannedStep>(v.clone()) {
            if !step.action.trim().is_empty() {
                steps.push(step);
            }
        }
    }
    if steps.is_empty() {
        return Ok(None);
    }
    Ok(Some((steps, model)))
}

fn strip_markdown_fence(input: &str) -> &str {
    let trimmed = input.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        let rest = rest.trim_start_matches("json").trim_start();
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim();
        }
    }
    trimmed
}
