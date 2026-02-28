use crate::local_auth::{
    issue_local_api_token, require_auth, revoke_local_api_token, AuthTokenIssueReq,
    AuthTokenResult, AuthTokenRevokeReq, AuthTokenRotateReq,
};
use crate::{
    append_audit, authorize_action, browser_navigate_inner, call_mcp_tool_inner, config_set_inner,
    config_show_inner, daemon_restart_inner, daemon_start_inner, daemon_stop_inner,
    ensure_action_still_authorized, fs_operate_inner, get_audit_events_inner,
    list_inbound_messages_inner, mail_inbox_unreplied_inner, memory_recent_inner,
    memory_search_inner, memory_store_inner, office_create_excel_inner,
    office_upload_graph_file_inner, policy_list_domain_inner, policy_set_domain_inner,
    send_message_inner, status_inner, task_run_inner, AppState, ApproveReq, BrowserNavigateApiReq,
    CommandError, ConfigSetReq, CreateSkillReq, FsOperateApiReq, InboundQuery, LimitQuery,
    MailInboxItem, MailInboxQuery, McpCallReq, MemorySearchReq, MemoryStoreReq, OfficeExcelApiReq,
    OfficeUploadApiReq, RunSkillReq, SendMessageReq, TaskRunReq,
};
use axum::extract::{Query, State as AxumState};
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use clawork_core::{
    ActionKind, ActionRequest, AuditEvent, BrowserRunResult, DomainPolicy, FsOperationResult,
    InboundMessage, OfficeUploadResult, SkillRequest, SkillResponse, SkillRuntime, TaskInfo,
};
use clawork_memory::{MemoryHit, MemoryRecord};
use clawork_skills::SkillManifest;
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
