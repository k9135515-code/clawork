use crate::local_auth::{
    issue_local_api_token, require_auth, revoke_local_api_token, AuthTokenIssueReq,
    AuthTokenResult, AuthTokenRevokeReq, AuthTokenRotateReq,
};
use crate::{
    append_audit, authorize_action, browser_navigate_inner, call_mcp_tool_inner, config_set_inner,
    config_show_inner, daemon_restart_inner, daemon_start_inner, daemon_stop_inner,
    ensure_action_still_authorized, fs_operate_inner, get_audit_events_inner,
    get_daily_briefing_inner, get_operator_store, list_inbound_messages_inner,
    list_proactive_suggestions_inner, mail_inbox_unreplied_inner, memory_recent_inner,
    memory_search_inner, memory_store_inner, office_create_excel_inner,
    office_upload_graph_file_inner, policy_list_domain_inner, policy_set_domain_inner,
    send_message_inner, status_inner, task_run_inner, truncate_text, AppState, ApproveReq,
    BrowserNavigateApiReq, CommandError, ConfigSetReq, CreateSkillReq, FsOperateApiReq,
    InboundQuery, LimitQuery, MailInboxItem, MailInboxQuery, McpCallReq, MemorySearchReq,
    MemoryStoreReq, OfficeExcelApiReq, OfficeUploadApiReq, RunSkillReq, SendMessageReq, TaskRunReq,
};
use axum::extract::{Query, State as AxumState};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use clawork_core::{
    ActionKind, ActionRequest, AuditEvent, BrowserRunRequest, BrowserRunResult, CitationRef,
    DomainPolicy, FsOperationKind, FsOperationRequest, FsOperationResult, InboundMessage,
    OfficeUploadResult, SkillRequest, SkillResponse, SkillRuntime, TaskInfo,
};
use clawork_memory::{MemoryHit, MemoryRecord};
use clawork_skills::SkillManifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
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

#[derive(Debug, Serialize)]
pub(crate) struct OpsHealth {
    started_at: chrono::DateTime<chrono::Utc>,
    uptime_seconds: i64,
    nl_requests: u64,
    nl_failures: u64,
    inbound_processed: u64,
    inbound_automations: u64,
    daemon_restarts: u64,
    last_error: Option<String>,
}

pub(crate) async fn api_ops_health(
    AxumState(state): AxumState<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<OpsHealth>, CommandError> {
    require_auth(&state, &headers)?;
    let snap = state.ops_runtime.read().clone();
    Ok(Json(OpsHealth {
        started_at: snap.started_at,
        uptime_seconds: (Utc::now() - snap.started_at).num_seconds().max(0),
        nl_requests: snap.nl_requests,
        nl_failures: snap.nl_failures,
        inbound_processed: snap.inbound_processed,
        inbound_automations: snap.inbound_automations,
        daemon_restarts: snap.daemon_restarts,
        last_error: snap.last_error,
    }))
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
    let result = nl_execute_inner(
        &state,
        body.instruction,
        body.dry_run.unwrap_or(false),
        body.continue_on_error.unwrap_or(false),
        body.approval_token,
    )
    .await?;
    Ok(Json(result))
}

pub(crate) async fn nl_execute_inner(
    state: &AppState,
    instruction: String,
    dry_run: bool,
    continue_on_error: bool,
    approval_token: Option<String>,
) -> Result<NlExecuteResult, CommandError> {
    {
        let mut ops = state.ops_runtime.write();
        ops.nl_requests += 1;
    }
    if instruction.trim().is_empty() {
        return Err(CommandError::validation("instruction must not be empty"));
    }

    let (steps, plan_source, model) = match build_llm_plan(state, &instruction).await {
        Ok(Some((steps, model))) => (steps, "llm".to_string(), Some(model)),
        _ => (build_rule_plan(&instruction), "rule".to_string(), None),
    };

    if steps.is_empty() {
        return Err(CommandError::validation(
            "could not derive executable steps from instruction",
        ));
    }

    let mut outcomes = Vec::<NlStepOutcome>::new();
    if !dry_run {
        for (idx, step) in steps.iter().enumerate() {
            match execute_planned_step(state, step, approval_token.clone()).await {
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
                    {
                        let mut ops = state.ops_runtime.write();
                        ops.nl_failures += 1;
                        ops.last_error = Some(format!("nl_execute step failed: {}", err.message));
                    }
                    if !continue_on_error {
                        return Err(err);
                    }
                }
            }
        }
    }

    Ok(NlExecuteResult {
        instruction,
        plan_source,
        model,
        dry_run,
        steps,
        outcomes,
    })
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
        "browser_autopilot" => {
            let target = param_str(params, "target")?;
            browser_autopilot_inner(state, target, approval_token).await
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
        "llm_prompt" => {
            let prompt = param_str(params, "prompt")?;
            let model = params
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string);
            llm_prompt_inner(state, prompt, model).await
        }
        "web_search" => {
            let query = param_str(params, "query")?;
            let limit = param_i64(params, "limit")
                .map(|v| v.max(1) as usize)
                .unwrap_or(5);
            web_search_inner(state, query, limit, approval_token).await
        }
        "connector_status" => {
            let store = get_operator_store(state)?;
            let statuses = store
                .connector_statuses()
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?;
            serde_json::to_value(statuses).map_err(|e| CommandError::internal(e.to_string()))
        }
        "workspace_snapshot" => {
            let store = get_operator_store(state)?;
            let statuses = store
                .connector_statuses()
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?;
            let connected = statuses.iter().filter(|s| s.connected).count();
            Ok(serde_json::json!({
                "ok": true,
                "total": statuses.len(),
                "connected": connected,
                "providers": statuses
            }))
        }
        "artifact_page" => {
            let title = param_str(params, "title")?;
            let body = param_str(params, "content")?;
            let project_id = params
                .get("project_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToOwned::to_owned);
            create_html_artifact_inner(state, title, body, project_id).await
        }
        other => Err(CommandError::validation(format!(
            "unsupported action '{other}'"
        ))),
    }
}

async fn llm_prompt_inner(
    state: &AppState,
    prompt: &str,
    model: Option<String>,
) -> Result<Value, CommandError> {
    let api_key = std::env::var("CLAWORK_OPENAI_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| CommandError::not_configured("CLAWORK_OPENAI_API_KEY is required"))?;
    let model = select_model_for_task("prompt", model);
    let payload = serde_json::json!({
        "model": model,
        "temperature": 0.2,
        "messages": [
            {"role":"system","content":"You are a concise assistant for local automation tasks."},
            {"role":"user","content": prompt}
        ]
    });
    let resp = state
        .http
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(40))
        .send()
        .await
        .map_err(|e| CommandError::internal(format!("llm request failed: {e}")))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| CommandError::internal(format!("llm response parse failed: {e}")))?;
    if !status.is_success() {
        return Err(CommandError::internal(format!(
            "llm failed ({}): {}",
            status.as_u16(),
            body
        )));
    }
    let content = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("message"))
        .and_then(|v| v.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(serde_json::json!({
        "ok": true,
        "model": model,
        "content": content
    }))
}

async fn web_search_inner(
    state: &AppState,
    query: &str,
    limit: usize,
    approval_token: Option<String>,
) -> Result<Value, CommandError> {
    let action = ActionRequest {
        kind: ActionKind::NetworkCall,
        target: Some("https://api.duckduckgo.com/".into()),
        params: serde_json::json!({ "query": query, "limit": limit }),
        trace_id: Uuid::new_v4(),
    };
    let ctx = authorize_action(state, &action, approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();

    let resp = state
        .http
        .get("https://api.duckduckgo.com/")
        .query(&[
            ("q", query),
            ("format", "json"),
            ("no_html", "1"),
            ("no_redirect", "1"),
        ])
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| CommandError::internal(format!("web search request failed: {e}")))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| CommandError::internal(format!("web search parse failed: {e}")))?;
    if !status.is_success() {
        return Err(CommandError::internal(format!(
            "web search failed ({}): {}",
            status.as_u16(),
            body
        )));
    }

    let mut results = Vec::<Value>::new();
    if let Some(topic) = body
        .get("AbstractText")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
    {
        results.push(serde_json::json!({
            "title": body.get("Heading").and_then(Value::as_str).unwrap_or("DuckDuckGo Instant Answer"),
            "url": body.get("AbstractURL").and_then(Value::as_str).unwrap_or(""),
            "snippet": topic
        }));
    }
    if let Some(related) = body.get("RelatedTopics").and_then(Value::as_array) {
        for item in related {
            if results.len() >= limit {
                break;
            }
            if let Some(text) = item.get("Text").and_then(Value::as_str) {
                results.push(serde_json::json!({
                    "title": text,
                    "url": item.get("FirstURL").and_then(Value::as_str).unwrap_or(""),
                    "snippet": text
                }));
                continue;
            }
            if let Some(topics) = item.get("Topics").and_then(Value::as_array) {
                for sub in topics {
                    if results.len() >= limit {
                        break;
                    }
                    if let Some(text) = sub.get("Text").and_then(Value::as_str) {
                        results.push(serde_json::json!({
                            "title": text,
                            "url": sub.get("FirstURL").and_then(Value::as_str).unwrap_or(""),
                            "snippet": text
                        }));
                    }
                }
            }
        }
    }
    results.truncate(limit);

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: "allowed".into(),
            reason: Some(format!(
                "mode={}, query_len={}, results={}",
                crate::permission_mode_label(&mode),
                query.len(),
                results.len()
            )),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(serde_json::json!({
        "ok": true,
        "query": query,
        "results": results
    }))
}

async fn browser_autopilot_inner(
    state: &AppState,
    target: &str,
    approval_token: Option<String>,
) -> Result<Value, CommandError> {
    let target = target.trim();
    if target.is_empty() {
        return Err(CommandError::validation("target is required"));
    }

    let mut candidates = Vec::<String>::new();
    if target.starts_with("http://") || target.starts_with("https://") {
        candidates.push(target.to_string());
    } else {
        candidates.push(format!("https://{target}"));
        let search = web_search_inner(state, target, 1, approval_token.clone()).await?;
        if let Some(url) = search
            .get("results")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|v| v.get("url"))
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
        {
            candidates.push(url.to_string());
        }
    }
    candidates.dedup();

    let mut last_error = None;
    for url in candidates {
        let req = BrowserRunRequest {
            url: url.clone(),
            allow_domains: vec![],
            headed: false,
            timeout_seconds: 30,
        };
        match browser_navigate_inner(state, req, approval_token.clone()).await {
            Ok(result) => {
                return Ok(serde_json::json!({
                    "ok": true,
                    "selected_url": url,
                    "result": result
                }));
            }
            Err(err) => {
                last_error = Some(err.message);
            }
        }
    }

    Err(CommandError::internal(format!(
        "browser_autopilot failed: {}",
        last_error.unwrap_or_else(|| "no candidates".into())
    )))
}

async fn create_html_artifact_inner(
    state: &AppState,
    title: &str,
    content: &str,
    project_id: Option<String>,
) -> Result<Value, CommandError> {
    let safe_title = truncate_text(title, 120);
    let body = truncate_text(content, 10_000);
    let now = Utc::now();
    let filename = format!("artifact-{}.html", now.format("%Y%m%d-%H%M%S").to_string());
    let base_dir = if let Some(pid) = &project_id {
        PathBuf::from("data").join("projects").join(pid)
    } else {
        PathBuf::from("data").join("artifacts")
    };
    tokio::fs::create_dir_all(&base_dir)
        .await
        .map_err(|e| CommandError::internal(format!("create artifact dir failed: {e}")))?;
    let path = base_dir.join(filename);
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title></head><body><h1>{}</h1><pre>{}</pre></body></html>",
        html_escape(&safe_title),
        html_escape(&safe_title),
        html_escape(&body)
    );
    tokio::fs::write(&path, html)
        .await
        .map_err(|e| CommandError::internal(format!("write artifact failed: {e}")))?;

    if let Some(pid) = project_id {
        if let Ok(store) = get_operator_store(state) {
            let _ = store
                .add_artifact(
                    &pid,
                    path.display().to_string(),
                    "text/html".to_string(),
                    Some("nl:artifact_page".to_string()),
                    Vec::<CitationRef>::new(),
                )
                .await;
        }
    }

    Ok(serde_json::json!({
        "ok": true,
        "path": path.display().to_string(),
        "mime": "text/html"
    }))
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
}

fn select_model_for_task(task: &str, preferred: Option<String>) -> String {
    if let Some(v) = preferred
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        return v;
    }
    let key = format!(
        "CLAWORK_OPENAI_MODEL_{}",
        task.replace('-', "_").to_ascii_uppercase()
    );
    if let Ok(v) = std::env::var(&key) {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    std::env::var("CLAWORK_OPENAI_CHAT_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string())
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
    if let Some(target) = text.strip_prefix("browse ") {
        return vec![NlPlannedStep {
            action: "browser_autopilot".into(),
            params: serde_json::json!({ "target": target.trim() }),
            rationale: Some("browse command matched".into()),
        }];
    }
    if let Some(q) = text.strip_prefix("search ") {
        return vec![NlPlannedStep {
            action: "web_search".into(),
            params: serde_json::json!({ "query": q.trim(), "limit": 5 }),
            rationale: Some("search command matched".into()),
        }];
    }
    if let Some(q) = text.strip_prefix("llm ") {
        return vec![NlPlannedStep {
            action: "llm_prompt".into(),
            params: serde_json::json!({ "prompt": q.trim() }),
            rationale: Some("llm command matched".into()),
        }];
    }
    if lower.contains("connector") || lower.contains("workspace") {
        return vec![NlPlannedStep {
            action: "workspace_snapshot".into(),
            params: serde_json::json!({}),
            rationale: Some("workspace/connector keyword matched".into()),
        }];
    }
    if let Some(rest) = text.strip_prefix("artifact ") {
        if let Some((title, body)) = rest.split_once("::") {
            return vec![NlPlannedStep {
                action: "artifact_page".into(),
                params: serde_json::json!({ "title": title.trim(), "content": body.trim() }),
                rationale: Some("artifact command matched".into()),
            }];
        }
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
    let model = select_model_for_task("planning", None);

    let system = "You are a planner for a local desktop agent. Return only JSON: {\"steps\":[{\"action\":\"status|briefing|suggestions|memory_store|memory_search|fs_read|fs_write|message_send|browser_navigate|browser_autopilot|mcp_call|llm_prompt|web_search|connector_status|workspace_snapshot|artifact_page|daemon_start|daemon_stop|daemon_restart|task_run\",\"params\":{},\"rationale\":\"...\"}]}. Use fs write format with params.path and params.content. For message_send include adapter,to,content.";
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
