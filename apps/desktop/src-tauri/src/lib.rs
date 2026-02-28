use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use clawork_adapters::{
    DiscordAdapter, IMessageAdapter, LineAdapter, SignalAdapter, SlackAdapter, TelegramAdapter,
    WhatsAppCloudAdapter,
};
use clawork_audit::AuditStore;
use clawork_browser::PlaywrightSandbox;
use clawork_capabilities::PermissionEngine;
use clawork_core::{
    ActionKind, ActionRequest, AppStatus, ApprovalProfile, AuditEvent, BriefingRationale,
    BrowserAutomationService, BrowserRunRequest, BrowserRunResult, CapabilityGuard, CitationRef,
    DailyBriefing, Decision, DomainPolicy, EmbeddingProvider, ErrorCode, FilesystemService,
    FsOperationKind, FsOperationRequest, FsOperationResult, InboundMessage, MessageAdapter,
    OfficeExcelRequest, OfficeGraphUploadRequest, OfficeUploadResult, OperatorActionProposal,
    OperatorActionState, OperatorSession, OperatorSessionState, OperatorStatus, OperatorStep,
    OperatorTaskTemplate, OperatorTimelineItem, OutboundMessage, PermissionMode,
    ProactiveSuggestion, ProjectArtifact, ProjectInfo, RequestContext, SkillRequest, SkillResponse,
    SkillRuntime, TaskInfo, ToolArgs, ToolCaller,
};
use clawork_daemon::{DaemonEvent, DaemonService};
use clawork_fs::SandboxFsService;
use clawork_llm::OpenAiEmbeddingProvider;
use clawork_mcp_client::StdioMcpClient;
use clawork_memory::{MemoryHit, MemoryRecord, MemoryStore};
use clawork_office::OfficeService;
use clawork_operator::{NewOperatorTask, OAuthCallbackInput, OperatorStore, ResearchJobRecord};
use clawork_skills::{SkillManifest, SkillRegistry, WasmSkillRuntime};
use parking_lot::RwLock;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

mod connectors_api;
mod local_api_handlers;
mod local_auth;
mod mcp_local;
mod mcp_remote;
mod operator_api;
mod research_api;
mod webhook_api;
use connectors_api::{
    api_connectors_google_drive_create, api_connectors_google_sheets_append,
    api_connectors_notion_page_create, api_connectors_notion_search, api_connectors_oauth_callback,
    api_connectors_oauth_start, api_connectors_slack_history, api_connectors_slack_post,
    api_connectors_status,
};
use local_api_handlers::{
    api_approve_action, api_auth_token_issue, api_auth_token_revoke, api_auth_token_rotate,
    api_browser_navigate, api_config_set, api_config_show, api_create_skill, api_daemon_restart,
    api_daemon_start, api_daemon_stop, api_fs_operate, api_get_status, api_get_tasks,
    api_inbound_messages, api_list_skills, api_logs, api_mail_inbox_unreplied, api_mcp_call,
    api_memory_recent, api_memory_search, api_memory_store, api_nl_execute, api_office_excel,
    api_office_upload, api_policy_list_domain, api_policy_set_domain, api_run_skill,
    api_send_message, api_task_run,
};
use local_auth::ensure_cli_token;
use mcp_local::maybe_call_local_connector_mcp_tool;
use mcp_remote::{call_remote_mcp_tool, mcp_target_for_route};
use operator_api::{
    api_operator_action_approve, api_operator_action_reject, api_operator_create_project,
    api_operator_create_session, api_operator_create_task, api_operator_get_session,
    api_operator_list_artifacts, api_operator_list_projects, api_operator_list_sessions,
    api_operator_list_tasks, api_operator_pending_approvals, api_operator_plan_session,
    api_operator_run_session, api_operator_run_task_now, api_operator_timeline,
};
use research_api::{
    api_briefing, api_media_image, api_media_video, api_research_create_job, api_research_get_job,
    api_research_get_report, api_suggestions,
};
use webhook_api::{api_telegram_webhook, api_whatsapp_webhook_event, api_whatsapp_webhook_verify};

#[derive(Clone)]
struct AdapterSet {
    telegram: Option<TelegramAdapter>,
    whatsapp: Option<WhatsAppCloudAdapter>,
    line: Option<LineAdapter>,
    discord: Option<DiscordAdapter>,
    slack: Option<SlackAdapter>,
    signal: Option<SignalAdapter>,
    imessage: Option<IMessageAdapter>,
}

#[derive(Clone)]
struct AppState {
    daemon: DaemonService,
    permissions: PermissionEngine,
    registry: SkillRegistry,
    runtime: WasmSkillRuntime,
    adapters: AdapterSet,
    audit: Arc<RwLock<Option<AuditStore>>>,
    fs: SandboxFsService,
    browser: PlaywrightSandbox,
    office: OfficeService,
    mcp: Option<StdioMcpClient>,
    memory: Arc<RwLock<Option<MemoryStore>>>,
    suggestions: Arc<RwLock<Vec<ProactiveSuggestion>>>,
    inbound_messages: Arc<RwLock<Vec<InboundMessage>>>,
    seen_inbound_ids: Arc<RwLock<VecDeque<String>>>,
    seen_inbound_path: PathBuf,
    daemon_should_run: Arc<RwLock<bool>>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    local_api_tokens: Arc<RwLock<HashMap<String, Option<chrono::DateTime<chrono::Utc>>>>>,
    local_api_addr: String,
    operator: Arc<RwLock<Option<OperatorStore>>>,
    init_errors: BTreeMap<String, String>,
    http: Client,
}

#[derive(Debug, Clone, Serialize)]
struct CommandError {
    code: ErrorCode,
    message: String,
    confirmation_token: Option<String>,
}

impl CommandError {
    fn denied(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Denied,
            message: message.into(),
            confirmation_token: None,
        }
    }

    fn confirmation(token: String, message: String) -> Self {
        Self {
            code: ErrorCode::ConfirmationRequired,
            message,
            confirmation_token: Some(token),
        }
    }

    fn not_configured(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::NotConfigured,
            message: message.into(),
            confirmation_token: None,
        }
    }

    fn validation(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::ValidationError,
            message: message.into(),
            confirmation_token: None,
        }
    }

    fn timeout(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Timeout,
            message: message.into(),
            confirmation_token: None,
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::InternalError,
            message: message.into(),
            confirmation_token: None,
        }
    }

    fn status_code(&self) -> StatusCode {
        match self.code {
            ErrorCode::Denied => StatusCode::FORBIDDEN,
            ErrorCode::ConfirmationRequired => StatusCode::CONFLICT,
            ErrorCode::NotConfigured => StatusCode::BAD_REQUEST,
            ErrorCode::ValidationError => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::Timeout => StatusCode::REQUEST_TIMEOUT,
            ErrorCode::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for CommandError {
    fn into_response(self) -> Response {
        (self.status_code(), Json(self)).into_response()
    }
}

#[derive(Debug, Deserialize)]
struct LimitQuery {
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct LimitQueryUsize {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct InboundQuery {
    limit: Option<usize>,
    adapter: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MailInboxQuery {
    limit: Option<usize>,
    approval_token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct MailInboxItem {
    id: String,
    thread_id: Option<String>,
    from: String,
    subject: String,
    date: Option<String>,
    snippet: String,
    reply_suggestion: String,
}

#[derive(Debug, Deserialize)]
struct WhatsAppWebhookVerifyQuery {
    #[serde(rename = "hub.mode")]
    mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    verify_token: Option<String>,
    #[serde(rename = "hub.challenge")]
    challenge: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramWebhookChat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct TelegramWebhookMessage {
    text: Option<String>,
    chat: TelegramWebhookChat,
}

#[derive(Debug, Deserialize)]
struct TelegramWebhookUpdate {
    update_id: i64,
    message: Option<TelegramWebhookMessage>,
}

#[derive(Debug, Deserialize)]
struct ApproveReq {
    token: String,
}

#[derive(Debug, Deserialize)]
struct RunSkillReq {
    skill_id: String,
    input: Value,
    approval_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateSkillReq {
    skill_id: String,
    name: String,
    description: Option<String>,
    approval_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SendMessageReq {
    adapter: String,
    to: String,
    content: String,
    approval_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FsOperateApiReq {
    #[serde(flatten)]
    op: FsOperationRequest,
    approval_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BrowserNavigateApiReq {
    #[serde(flatten)]
    req: BrowserRunRequest,
    approval_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OfficeExcelApiReq {
    #[serde(flatten)]
    req: OfficeExcelRequest,
    approval_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OfficeUploadApiReq {
    #[serde(flatten)]
    req: OfficeGraphUploadRequest,
    approval_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct McpCallReq {
    tool_name: String,
    payload: Value,
    route: Option<String>,
    approval_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MemoryStoreReq {
    text: String,
    embedding: Option<Vec<f32>>,
    approval_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MemorySearchReq {
    query: String,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TaskRunReq {
    task_id: String,
}

#[derive(Debug, Deserialize)]
struct ConfigSetReq {
    key: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct OperatorCreateSessionReq {
    title: String,
    goal: String,
}

#[derive(Debug, Deserialize)]
struct OperatorPlanReq {
    steps: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OperatorActionStateReq {
    actor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OperatorTaskCreateReq {
    name: String,
    cron: String,
    prompt: String,
    target_project: Option<String>,
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct OperatorProjectCreateReq {
    name: String,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConnectorOAuthStartReq {
    redirect_uri: Option<String>,
    scopes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct ConnectorOAuthCallbackReq {
    account_id: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    scopes: Option<Vec<String>>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleSheetsAppendReq {
    spreadsheet_id: String,
    sheet_name: Option<String>,
    values: Vec<Vec<String>>,
    value_input_option: Option<String>,
    approval_token: Option<String>,
}

#[derive(Debug, Serialize)]
struct GoogleSheetsAppendResult {
    ok: bool,
    spreadsheet_id: String,
    updated_range: Option<String>,
    updated_rows: Option<i64>,
    updated_cells: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct GoogleDriveCreateReq {
    name: String,
    parent_id: Option<String>,
    mime_type: Option<String>,
    content: String,
    approval_token: Option<String>,
}

#[derive(Debug, Serialize)]
struct GoogleDriveCreateResult {
    ok: bool,
    id: Option<String>,
    name: Option<String>,
    mime_type: Option<String>,
    web_view_link: Option<String>,
    web_content_link: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NotionSearchReq {
    query: String,
    page_size: Option<usize>,
    approval_token: Option<String>,
}

#[derive(Debug, Serialize)]
struct NotionSearchResult {
    ok: bool,
    query: String,
    total: usize,
    results: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct NotionPageCreateReq {
    title: String,
    content: Option<String>,
    parent_page_id: Option<String>,
    parent_database_id: Option<String>,
    approval_token: Option<String>,
}

#[derive(Debug, Serialize)]
struct NotionPageCreateResult {
    ok: bool,
    id: Option<String>,
    url: Option<String>,
    created_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlackPostReq {
    channel: String,
    text: String,
    approval_token: Option<String>,
}

#[derive(Debug, Serialize)]
struct SlackPostResult {
    ok: bool,
    channel: Option<String>,
    ts: Option<String>,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlackHistoryReq {
    channel: String,
    limit: Option<usize>,
    approval_token: Option<String>,
}

#[derive(Debug, Serialize)]
struct SlackHistoryResult {
    ok: bool,
    channel: String,
    messages: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct ResearchJobCreateReq {
    title: Option<String>,
    question: String,
    source_urls: Vec<String>,
    project_id: Option<String>,
    approval_token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ResearchJobReport {
    generated_at: chrono::DateTime<chrono::Utc>,
    question: String,
    summary: String,
    findings: Vec<String>,
    citations: Vec<Value>,
    sources: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct MediaGenerateReq {
    prompt: String,
    provider: Option<String>,
    model: Option<String>,
    size: Option<String>,
    seconds: Option<u32>,
    project_id: Option<String>,
    output_path: Option<String>,
    approval_token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct MediaGenerateResult {
    ok: bool,
    kind: String,
    provider: String,
    model: Option<String>,
    output_path: String,
    mime_type: String,
    remote_url: Option<String>,
    artifact_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DomainPolicySetReq {
    domain: String,
    profile: ApprovalProfile,
    blocked_actions: Vec<ActionKind>,
    allow_actions: Vec<ActionKind>,
}

#[derive(Debug, Deserialize)]
struct LimitQueryDefault {
    limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
struct GenericOk {
    ok: bool,
    message: String,
}

#[tauri::command]
async fn get_status(state: State<'_, AppState>) -> Result<AppStatus, CommandError> {
    Ok(status_inner(&state).await)
}

#[tauri::command]
async fn list_tasks(state: State<'_, AppState>) -> Result<Vec<TaskInfo>, CommandError> {
    Ok(state.daemon.list_tasks())
}

#[tauri::command]
async fn request_elevation(
    state: State<'_, AppState>,
    ttl_seconds: Option<i64>,
) -> Result<PermissionMode, CommandError> {
    let mode = state
        .permissions
        .request_elevation(ttl_seconds.unwrap_or(300));

    let audit = AuditEvent {
        timestamp: Utc::now(),
        actor: "desktop_ui".into(),
        action: ActionKind::ElevatedRequest,
        target: Some("local-session".into()),
        decision: "allowed".into(),
        reason: Some(format!(
            "explicit user request, mode={}",
            permission_mode_label(&mode)
        )),
        trace_id: Uuid::new_v4(),
    };
    append_audit(&state, &audit).await;

    Ok(mode)
}

#[tauri::command]
async fn approve_action(state: State<'_, AppState>, token: String) -> Result<bool, CommandError> {
    let approved = state.permissions.approve_token(&token);
    append_audit(
        &state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: "desktop_ui".into(),
            action: ActionKind::ElevatedRequest,
            target: Some("approval-token".into()),
            decision: if approved {
                "allowed".into()
            } else {
                "denied".into()
            },
            reason: Some(format!("token={token}")),
            trace_id: Uuid::new_v4(),
        },
    )
    .await;
    Ok(approved)
}

#[tauri::command]
async fn list_skills(state: State<'_, AppState>) -> Result<Vec<SkillManifest>, CommandError> {
    Ok(state.registry.list())
}

#[tauri::command]
async fn run_skill(
    state: State<'_, AppState>,
    skill_id: String,
    input: Value,
    approval_token: Option<String>,
) -> Result<SkillResponse, CommandError> {
    let action = ActionRequest {
        kind: ActionKind::SkillExecute,
        target: Some(skill_id.clone()),
        params: input.clone(),
        trace_id: Uuid::new_v4(),
    };

    let ctx = authorize_action(&state, &action, approval_token).await?;
    ensure_action_still_authorized(&state, &action, &ctx).await?;

    let response = state
        .runtime
        .execute(
            &skill_id,
            SkillRequest {
                abi_version: "v1".into(),
                skill_id: skill_id.clone(),
                input,
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

    Ok(response)
}

#[tauri::command]
async fn create_skill(
    state: State<'_, AppState>,
    skill_id: String,
    name: String,
    description: Option<String>,
    approval_token: Option<String>,
) -> Result<SkillManifest, CommandError> {
    let action = ActionRequest {
        kind: ActionKind::SkillInstall,
        target: Some(skill_id.clone()),
        params: serde_json::json!({ "name": name.clone(), "description": description.clone() }),
        trace_id: Uuid::new_v4(),
    };

    let ctx = authorize_action(&state, &action, approval_token).await?;
    ensure_action_still_authorized(&state, &action, &ctx).await?;
    let manifest = state
        .registry
        .create_skill_template(&skill_id, &name, description)
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

    Ok(manifest)
}

#[tauri::command]
async fn send_message(
    state: State<'_, AppState>,
    adapter: String,
    to: String,
    content: String,
    approval_token: Option<String>,
) -> Result<clawork_core::SendResult, CommandError> {
    send_message_inner(&state, adapter, to, content, approval_token).await
}

#[tauri::command]
async fn get_audit_events(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<AuditEvent>, CommandError> {
    get_audit_events_inner(&state, limit).await
}

#[tauri::command]
async fn fs_operate(
    state: State<'_, AppState>,
    op: FsOperationRequest,
    approval_token: Option<String>,
) -> Result<FsOperationResult, CommandError> {
    fs_operate_inner(&state, op, approval_token).await
}

#[tauri::command]
async fn browser_navigate(
    state: State<'_, AppState>,
    req: BrowserRunRequest,
    approval_token: Option<String>,
) -> Result<BrowserRunResult, CommandError> {
    browser_navigate_inner(&state, req, approval_token).await
}

#[tauri::command]
async fn call_mcp_tool(
    state: State<'_, AppState>,
    tool_name: String,
    payload: Value,
    route: Option<String>,
    approval_token: Option<String>,
) -> Result<clawork_core::ToolResult, CommandError> {
    call_mcp_tool_inner(&state, tool_name, payload, route, approval_token).await
}

#[tauri::command]
async fn office_create_excel(
    state: State<'_, AppState>,
    req: OfficeExcelRequest,
    approval_token: Option<String>,
) -> Result<String, CommandError> {
    office_create_excel_inner(&state, req, approval_token).await
}

#[tauri::command]
async fn office_upload_graph_file(
    state: State<'_, AppState>,
    req: OfficeGraphUploadRequest,
    approval_token: Option<String>,
) -> Result<OfficeUploadResult, CommandError> {
    office_upload_graph_file_inner(&state, req, approval_token).await
}

#[tauri::command]
async fn memory_store(
    state: State<'_, AppState>,
    text: String,
    embedding: Option<Vec<f32>>,
    approval_token: Option<String>,
) -> Result<String, CommandError> {
    memory_store_inner(&state, text, embedding, approval_token).await
}

#[tauri::command]
async fn memory_recent(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<MemoryRecord>, CommandError> {
    memory_recent_inner(&state, limit).await
}

#[tauri::command]
async fn memory_search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<MemoryHit>, CommandError> {
    memory_search_inner(&state, query, limit).await
}

#[tauri::command]
async fn list_proactive_suggestions(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<ProactiveSuggestion>, CommandError> {
    list_proactive_suggestions_inner(&state, limit).await
}

#[tauri::command]
async fn get_inbound_messages(
    state: State<'_, AppState>,
    limit: Option<usize>,
    adapter: Option<String>,
) -> Result<Vec<InboundMessage>, CommandError> {
    list_inbound_messages_inner(&state, limit, adapter).await
}

#[tauri::command]
async fn get_daily_briefing(state: State<'_, AppState>) -> Result<DailyBriefing, CommandError> {
    get_daily_briefing_inner(&state).await
}

#[tauri::command]
async fn operator_create_session(
    state: State<'_, AppState>,
    title: String,
    goal: String,
) -> Result<OperatorSession, CommandError> {
    operator_create_session_inner(&state, title, goal).await
}

#[tauri::command]
async fn operator_list_sessions(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<OperatorSession>, CommandError> {
    operator_list_sessions_inner(&state, limit).await
}

#[tauri::command]
async fn operator_plan_session(
    state: State<'_, AppState>,
    session_id: String,
    steps: Vec<String>,
) -> Result<Vec<OperatorStep>, CommandError> {
    operator_plan_session_inner(&state, session_id, steps).await
}

#[tauri::command]
async fn operator_run_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<OperatorSession, CommandError> {
    operator_run_session_inner(&state, session_id).await
}

#[tauri::command]
async fn operator_timeline(
    state: State<'_, AppState>,
    session_id: String,
    limit: Option<usize>,
) -> Result<Vec<OperatorTimelineItem>, CommandError> {
    operator_timeline_inner(&state, session_id, limit).await
}

#[tauri::command]
async fn operator_pending_approvals(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<OperatorActionProposal>, CommandError> {
    operator_pending_approvals_inner(&state, limit).await
}

#[tauri::command]
async fn operator_approve_proposal(
    state: State<'_, AppState>,
    action_id: String,
    actor: Option<String>,
) -> Result<OperatorActionProposal, CommandError> {
    operator_set_action_state_inner(&state, action_id, OperatorActionState::Approved, actor).await
}

#[tauri::command]
async fn operator_reject_proposal(
    state: State<'_, AppState>,
    action_id: String,
    actor: Option<String>,
) -> Result<OperatorActionProposal, CommandError> {
    operator_set_action_state_inner(&state, action_id, OperatorActionState::Rejected, actor).await
}

#[tauri::command]
async fn operator_create_task(
    state: State<'_, AppState>,
    name: String,
    cron: String,
    prompt: String,
    target_project: Option<String>,
    enabled: Option<bool>,
) -> Result<OperatorTaskTemplate, CommandError> {
    operator_create_task_inner(
        &state,
        OperatorTaskCreateReq {
            name,
            cron,
            prompt,
            target_project,
            enabled,
        },
    )
    .await
}

#[tauri::command]
async fn operator_list_tasks(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<OperatorTaskTemplate>, CommandError> {
    operator_list_tasks_inner(&state, limit).await
}

async fn status_inner(state: &AppState) -> AppStatus {
    let snap = state.daemon.snapshot();
    let operator = {
        let store = {
            let guard = state.operator.read();
            guard.clone()
        };
        if let Some(store) = store {
            if let Ok((active_sessions, pending_approvals, scheduled_tasks, connectors_health)) =
                store.operator_status_counts().await
            {
                Some(OperatorStatus {
                    active_sessions,
                    pending_approvals,
                    scheduled_tasks,
                    connectors_health,
                })
            } else {
                None
            }
        } else {
            None
        }
    };

    AppStatus {
        app_mode: state.permissions.current_mode(),
        daemon_running: snap.running,
        last_heartbeat: snap.last_heartbeat,
        loaded_skills: state.registry.list().len(),
        adapters: adapter_status_map(&state.adapters),
        operator,
        init_errors: state.init_errors.clone(),
    }
}

fn adapter_status_map(adapters: &AdapterSet) -> BTreeMap<String, bool> {
    let mut map = BTreeMap::new();
    map.insert("telegram".into(), adapters.telegram.is_some());
    map.insert("whatsapp".into(), adapters.whatsapp.is_some());
    map.insert("line".into(), adapters.line.is_some());
    map.insert("discord".into(), adapters.discord.is_some());
    map.insert("slack".into(), adapters.slack.is_some());
    map.insert("signal".into(), adapters.signal.is_some());

    let expose_unsupported = std::env::var("CLAWORK_EXPOSE_UNSUPPORTED_ADAPTERS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if cfg!(target_os = "macos") || expose_unsupported || adapters.imessage.is_some() {
        map.insert("imessage".into(), adapters.imessage.is_some());
    }

    map
}

async fn send_message_inner(
    state: &AppState,
    adapter: String,
    to: String,
    content: String,
    approval_token: Option<String>,
) -> Result<clawork_core::SendResult, CommandError> {
    let action = ActionRequest {
        kind: ActionKind::MessageSend,
        target: Some(format!("{adapter}:{to}")),
        params: serde_json::json!({ "content_len": content.len() }),
        trace_id: Uuid::new_v4(),
    };

    let ctx = authorize_action(state, &action, approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();
    let outbound = OutboundMessage {
        adapter: adapter.clone(),
        to,
        content,
    };

    let result = match adapter.as_str() {
        "telegram" => {
            let Some(client) = &state.adapters.telegram else {
                return Err(CommandError::not_configured(
                    "telegram adapter is not configured",
                ));
            };
            client
                .send(outbound)
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?
        }
        "whatsapp" => {
            let Some(client) = &state.adapters.whatsapp else {
                return Err(CommandError::not_configured(
                    "whatsapp adapter is not configured",
                ));
            };
            client
                .send(outbound)
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?
        }
        "line" => {
            let Some(client) = &state.adapters.line else {
                return Err(CommandError::not_configured(
                    "line adapter is not configured",
                ));
            };
            client
                .send(outbound)
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?
        }
        "discord" => {
            let Some(client) = &state.adapters.discord else {
                return Err(CommandError::not_configured(
                    "discord adapter is not configured",
                ));
            };
            client
                .send(outbound)
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?
        }
        "slack" => {
            let Some(client) = &state.adapters.slack else {
                return Err(CommandError::not_configured(
                    "slack adapter is not configured",
                ));
            };
            if !client.send_enabled() {
                return Err(CommandError::not_configured(
                    "slack webhook is not configured (set CLAWORK_SLACK_WEBHOOK_URL)",
                ));
            }
            client
                .send(outbound)
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?
        }
        "signal" => {
            let Some(client) = &state.adapters.signal else {
                return Err(CommandError::not_configured(
                    "signal adapter is not configured",
                ));
            };
            client
                .send(outbound)
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?
        }
        "imessage" => {
            let Some(client) = &state.adapters.imessage else {
                return Err(CommandError::not_configured(
                    "imessage adapter is not configured",
                ));
            };
            client
                .send(outbound)
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?
        }
        _ => {
            return Err(CommandError::validation("unsupported adapter"));
        }
    };

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: if result.ok {
                "allowed".into()
            } else {
                "error".into()
            },
            reason: Some(format!(
                "mode={}, send_error={}",
                permission_mode_label(&mode),
                result.error.clone().unwrap_or_default()
            )),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(result)
}

async fn get_audit_events_inner(
    state: &AppState,
    limit: Option<i64>,
) -> Result<Vec<AuditEvent>, CommandError> {
    let store = {
        let guard = state.audit.read();
        guard.clone()
    };

    if let Some(store) = store {
        store
            .list_recent(limit.unwrap_or(50))
            .await
            .map_err(|e| CommandError::internal(e.to_string()))
    } else {
        Ok(vec![])
    }
}

async fn fs_operate_inner(
    state: &AppState,
    op: FsOperationRequest,
    approval_token: Option<String>,
) -> Result<FsOperationResult, CommandError> {
    let op_kind = op.kind.clone();
    let action_kind = match op_kind {
        FsOperationKind::ReadFile | FsOperationKind::ListDirectory => ActionKind::FileRead,
        FsOperationKind::WriteFile
        | FsOperationKind::DeleteFile
        | FsOperationKind::CreateDirectory => ActionKind::FileWrite,
    };

    let action = ActionRequest {
        kind: action_kind,
        target: Some(op.path.clone()),
        params: serde_json::json!({ "op": op_kind }),
        trace_id: Uuid::new_v4(),
    };

    let ctx = authorize_action(state, &action, approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();
    let result = state
        .fs
        .operate(op, mode.clone())
        .await
        .map_err(|e| CommandError::validation(e.to_string()))?;

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: "allowed".into(),
            reason: Some(format!("mode={}", permission_mode_label(&mode))),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(result)
}

async fn browser_navigate_inner(
    state: &AppState,
    req: BrowserRunRequest,
    approval_token: Option<String>,
) -> Result<BrowserRunResult, CommandError> {
    let action = ActionRequest {
        kind: ActionKind::BrowserNav,
        target: Some(req.url.clone()),
        params: serde_json::json!({ "allow_domains": req.allow_domains }),
        trace_id: Uuid::new_v4(),
    };

    let ctx = authorize_action(state, &action, approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();
    let result = state.browser.navigate(req).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("timed out") {
            CommandError::timeout(msg)
        } else {
            CommandError::internal(msg)
        }
    })?;

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: if result.ok {
                "allowed".into()
            } else {
                "error".into()
            },
            reason: if result.ok {
                Some(format!("mode={}", permission_mode_label(&mode)))
            } else {
                Some(format!(
                    "mode={}, stderr={}",
                    permission_mode_label(&mode),
                    result.stderr
                ))
            },
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(result)
}

async fn call_mcp_tool_inner(
    state: &AppState,
    tool_name: String,
    payload: Value,
    route: Option<String>,
    approval_token: Option<String>,
) -> Result<clawork_core::ToolResult, CommandError> {
    let route = route
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("local")
        .to_lowercase();

    if route == "local" {
        if let Some(result) =
            maybe_call_local_connector_mcp_tool(state, &tool_name, &payload, approval_token.clone())
                .await?
        {
            return Ok(result);
        }
    }

    let target = mcp_target_for_route(&route, &tool_name)?;

    let action = ActionRequest {
        kind: ActionKind::NetworkCall,
        target: Some(target.clone()),
        params: payload.clone(),
        trace_id: Uuid::new_v4(),
    };

    let ctx = authorize_action(state, &action, approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();
    let result = if route == "remote" {
        call_remote_mcp_tool(state, &tool_name, payload, target).await?
    } else {
        let Some(mcp) = &state.mcp else {
            return Err(CommandError::not_configured(
                "MCP client not configured (set CLAWORK_MCP_COMMAND)",
            ));
        };
        mcp.call(&tool_name, ToolArgs { payload })
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("timed out") {
                    CommandError::timeout(msg)
                } else {
                    CommandError::internal(msg)
                }
            })?
    };

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: if result.ok {
                "allowed".into()
            } else {
                "error".into()
            },
            reason: Some(format!(
                "mode={}, route={}, mcp_error={}",
                permission_mode_label(&mode),
                route,
                result.error.clone().unwrap_or_default()
            )),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(result)
}

async fn office_create_excel_inner(
    state: &AppState,
    req: OfficeExcelRequest,
    approval_token: Option<String>,
) -> Result<String, CommandError> {
    let action = ActionRequest {
        kind: ActionKind::FileWrite,
        target: Some(req.output_path.clone()),
        params: serde_json::json!({ "sheet": req.sheet_name }),
        trace_id: Uuid::new_v4(),
    };

    let ctx = authorize_action(state, &action, approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();
    let out = state
        .office
        .create_excel_report(req)
        .await
        .map_err(|e| CommandError::validation(e.to_string()))?;

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: "allowed".into(),
            reason: Some(format!("mode={}", permission_mode_label(&mode))),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(out)
}

async fn office_upload_graph_file_inner(
    state: &AppState,
    req: OfficeGraphUploadRequest,
    approval_token: Option<String>,
) -> Result<OfficeUploadResult, CommandError> {
    let action = ActionRequest {
        kind: ActionKind::NetworkCall,
        target: Some(req.remote_path.clone()),
        params: serde_json::json!({ "mime": req.mime_type }),
        trace_id: Uuid::new_v4(),
    };

    let ctx = authorize_action(state, &action, approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();
    let result = state
        .office
        .upload_file_to_graph(req)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: if result.ok {
                "allowed".into()
            } else {
                "error".into()
            },
            reason: if result.ok {
                Some(format!("mode={}", permission_mode_label(&mode)))
            } else {
                Some(format!(
                    "mode={}, response={}",
                    permission_mode_label(&mode),
                    result.response_body
                ))
            },
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(result)
}

async fn memory_store_inner(
    state: &AppState,
    text: String,
    embedding: Option<Vec<f32>>,
    approval_token: Option<String>,
) -> Result<String, CommandError> {
    let action = ActionRequest {
        kind: ActionKind::FileWrite,
        target: Some("memory:store".into()),
        params: serde_json::json!({ "text_len": text.len() }),
        trace_id: Uuid::new_v4(),
    };

    let ctx = authorize_action(state, &action, approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let memory = get_memory_store(state)?;

    let embedded = if let Some(v) = embedding {
        v
    } else {
        embedding_for_text(state, &text).await
    };

    let id = memory
        .store_text(&text, Some(&embedded))
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: Some(format!("memory:{id}")),
            decision: "allowed".into(),
            reason: None,
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(id)
}

async fn memory_recent_inner(
    state: &AppState,
    limit: Option<i64>,
) -> Result<Vec<MemoryRecord>, CommandError> {
    let memory = get_memory_store(state)?;
    memory
        .recent(limit.unwrap_or(20))
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

async fn memory_search_inner(
    state: &AppState,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<MemoryHit>, CommandError> {
    let memory = get_memory_store(state)?;
    let max = limit.unwrap_or(10);

    let embedding = embedding_for_text(state, &query).await;
    let hits = memory
        .search_by_embedding(&embedding, max)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;

    if !hits.is_empty() {
        return Ok(hits);
    }

    let fallback = memory
        .search_like(&query, max)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;
    Ok(fallback
        .into_iter()
        .map(|record| MemoryHit { record, score: 0.0 })
        .collect())
}

async fn list_proactive_suggestions_inner(
    state: &AppState,
    limit: Option<usize>,
) -> Result<Vec<ProactiveSuggestion>, CommandError> {
    let max = limit.unwrap_or(20);
    let list = state.suggestions.read();
    let n = list.len();
    let start = n.saturating_sub(max);
    Ok(list[start..].to_vec())
}

async fn get_daily_briefing_inner(state: &AppState) -> Result<DailyBriefing, CommandError> {
    let tasks = state.daemon.list_tasks();
    let suggestions = {
        let list = state.suggestions.read();
        let n = list.len();
        let start = n.saturating_sub(5);
        list[start..].to_vec()
    };

    let audit_events = {
        let store = {
            let guard = state.audit.read();
            guard.clone()
        };
        if let Some(store) = store {
            store
                .list_recent(20)
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?
        } else {
            vec![]
        }
    };

    let memory_entries = {
        let mem = {
            let guard = state.memory.read();
            guard.clone()
        };
        if let Some(mem) = mem {
            mem.count().await.unwrap_or(0) as usize
        } else {
            0
        }
    };

    Ok(DailyBriefing {
        generated_at: Utc::now(),
        overview: format!(
            "{} scheduled tasks, {} recent audits, {} memories.",
            tasks.len(),
            audit_events.len(),
            memory_entries
        ),
        tasks_due: tasks.len(),
        recent_audit_events: audit_events.len(),
        memory_entries,
        rationales: vec![
            format!("tasks_due={} from daemon task registry", tasks.len()),
            format!(
                "recent_audit_events={} from latest audit rows",
                audit_events.len()
            ),
            format!("memory_entries={} from memory row count", memory_entries),
            format!(
                "suggestions={} from proactive suggestion buffer",
                suggestions.len()
            ),
        ],
        rationale_sources: vec![
            BriefingRationale {
                source: "daemon.tasks".into(),
                value: tasks.len().to_string(),
                reason: "count of registered daemon tasks".into(),
            },
            BriefingRationale {
                source: "audit.recent".into(),
                value: audit_events.len().to_string(),
                reason: "latest audit rows sampled for briefing".into(),
            },
            BriefingRationale {
                source: "memory.count".into(),
                value: memory_entries.to_string(),
                reason: "current memory row count".into(),
            },
            BriefingRationale {
                source: "suggestions.buffer".into(),
                value: suggestions.len().to_string(),
                reason: "cached proactive suggestions window".into(),
            },
        ],
        suggestions,
    })
}

async fn poll_adapters_once(state: &AppState) -> Result<Vec<InboundMessage>, CommandError> {
    let mut inbound = Vec::new();
    if let Some(adapter) = &state.adapters.telegram {
        inbound.extend(
            adapter
                .poll()
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?,
        );
    }
    if let Some(adapter) = &state.adapters.whatsapp {
        inbound.extend(
            adapter
                .poll()
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?,
        );
    }
    if let Some(adapter) = &state.adapters.line {
        inbound.extend(
            adapter
                .poll()
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?,
        );
    }
    if let Some(adapter) = &state.adapters.discord {
        inbound.extend(
            adapter
                .poll()
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?,
        );
    }
    if let Some(adapter) = &state.adapters.slack {
        inbound.extend(
            adapter
                .poll()
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?,
        );
    }
    if let Some(adapter) = &state.adapters.signal {
        inbound.extend(
            adapter
                .poll()
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?,
        );
    }
    if let Some(adapter) = &state.adapters.imessage {
        inbound.extend(
            adapter
                .poll()
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?,
        );
    }
    Ok(inbound)
}

fn remember_inbound_messages(
    state: &AppState,
    incoming: Vec<InboundMessage>,
) -> Vec<InboundMessage> {
    const MAX_SEEN: usize = 2048;
    const MAX_BUFFER: usize = 1000;

    let mut seen = state.seen_inbound_ids.write();
    let mut buffer = state.inbound_messages.write();
    let mut accepted = Vec::new();
    let mut newly_seen_keys = Vec::new();

    for msg in incoming {
        let dedupe_key = msg
            .external_id
            .as_ref()
            .map(|id| format!("{}:{}", msg.adapter, id));
        if let Some(key) = dedupe_key {
            if seen.contains(&key) {
                continue;
            }
            newly_seen_keys.push(key.clone());
            seen.push_back(key);
            while seen.len() > MAX_SEEN {
                seen.pop_front();
            }
        }
        buffer.push(msg.clone());
        accepted.push(msg);
    }

    if buffer.len() > MAX_BUFFER {
        let drop_n = buffer.len() - MAX_BUFFER;
        buffer.drain(0..drop_n);
    }

    let compact_snapshot = if !newly_seen_keys.is_empty() && seen.len() % 256 == 0 {
        Some(seen.clone())
    } else {
        None
    };
    drop(buffer);
    drop(seen);
    persist_seen_inbound_keys(
        &state.seen_inbound_path,
        &newly_seen_keys,
        compact_snapshot.as_ref(),
    );

    accepted
}

async fn ingest_inbound_messages(
    state: &AppState,
    actor: &str,
    incoming: Vec<InboundMessage>,
) -> usize {
    let accepted = remember_inbound_messages(state, incoming);
    if accepted.is_empty() {
        return 0;
    }

    for msg in &accepted {
        append_audit(
            state,
            &AuditEvent {
                timestamp: Utc::now(),
                actor: actor.to_string(),
                action: ActionKind::NetworkCall,
                target: Some(format!("{}:{}", msg.adapter, msg.from)),
                decision: "allowed".into(),
                reason: Some(format!(
                    "inbound message captured external_id={}",
                    msg.external_id.clone().unwrap_or_default()
                )),
                trace_id: Uuid::new_v4(),
            },
        )
        .await;
    }

    push_suggestion(
        &state.suggestions,
        ProactiveSuggestion {
            at: Utc::now(),
            text: format!("{} new inbound messages received.", accepted.len()),
        },
    );
    accepted.len()
}

async fn list_inbound_messages_inner(
    state: &AppState,
    limit: Option<usize>,
    adapter: Option<String>,
) -> Result<Vec<InboundMessage>, CommandError> {
    let polled = poll_adapters_once(state).await?;
    let _ = ingest_inbound_messages(state, "adapter-poller", polled).await;

    let max = limit.unwrap_or(50);
    let mut messages = state.inbound_messages.read().clone();
    if let Some(adapter) = adapter {
        messages.retain(|m| m.adapter.eq_ignore_ascii_case(&adapter));
    }
    messages.reverse();
    messages.truncate(max);
    Ok(messages)
}

async fn mail_inbox_unreplied_inner(
    state: &AppState,
    limit: Option<usize>,
    approval_token: Option<String>,
) -> Result<Vec<MailInboxItem>, CommandError> {
    let max = limit.unwrap_or(10).clamp(1, 50);
    let query = "in:inbox is:unread -from:me";
    let action = ActionRequest {
        kind: ActionKind::NetworkCall,
        target: Some("https://gmail.googleapis.com/gmail/v1/users/me/messages".into()),
        params: serde_json::json!({
            "query": query,
            "limit": max
        }),
        trace_id: Uuid::new_v4(),
    };

    let ctx = authorize_action(state, &action, approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();
    let token = google_connector_access_token(state).await?;

    let list_resp = state
        .http
        .get("https://gmail.googleapis.com/gmail/v1/users/me/messages")
        .query(&[
            ("q", query),
            ("maxResults", &max.to_string()),
            ("includeSpamTrash", "false"),
        ])
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| CommandError::internal(format!("gmail list request failed: {e}")))?;

    let status = list_resp.status();
    let list_body: Value = list_resp
        .json()
        .await
        .map_err(|e| CommandError::internal(format!("gmail list response parse failed: {e}")))?;
    if !status.is_success() {
        return Err(CommandError::internal(format!(
            "gmail list failed ({}): {}",
            status.as_u16(),
            list_body
        )));
    }

    let messages = list_body
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for row in messages {
        let Some(id) = row.get("id").and_then(Value::as_str) else {
            continue;
        };
        let detail_resp = state
            .http
            .get(format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages/{id}"
            ))
            .query(&[
                ("format", "metadata"),
                ("metadataHeaders", "From"),
                ("metadataHeaders", "Subject"),
                ("metadataHeaders", "Date"),
            ])
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| CommandError::internal(format!("gmail detail request failed: {e}")))?;
        if !detail_resp.status().is_success() {
            continue;
        }
        let detail: Value = detail_resp
            .json()
            .await
            .map_err(|e| CommandError::internal(format!("gmail detail parse failed: {e}")))?;

        let headers = detail
            .get("payload")
            .and_then(|v| v.get("headers"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let from = find_gmail_header(&headers, "From").unwrap_or_else(|| "unknown".into());
        let subject =
            find_gmail_header(&headers, "Subject").unwrap_or_else(|| "(no subject)".into());
        let date = find_gmail_header(&headers, "Date");
        let snippet = detail
            .get("snippet")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();

        let reply_suggestion = build_reply_suggestion(&from, &subject, &snippet);
        out.push(MailInboxItem {
            id: id.to_string(),
            thread_id: detail
                .get("threadId")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            from,
            subject,
            date,
            snippet,
            reply_suggestion,
        });

        if out.len() >= max {
            break;
        }
    }

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: "allowed".into(),
            reason: Some(format!(
                "mode={}, unread_count={}",
                permission_mode_label(&mode),
                out.len()
            )),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(out)
}

fn persist_seen_inbound_keys(
    path: &Path,
    new_keys: &[String],
    compact_snapshot: Option<&VecDeque<String>>,
) {
    if new_keys.is_empty() {
        return;
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        for key in new_keys {
            let _ = writeln!(file, "{key}");
        }
    }

    if let Some(snapshot) = compact_snapshot {
        let body = snapshot
            .iter()
            .map(|key| format!("{key}\n"))
            .collect::<String>();
        let _ = std::fs::write(path, body);
    }
}

async fn daemon_start_inner(state: &AppState) -> Result<GenericOk, CommandError> {
    *state.daemon_should_run.write() = true;
    state
        .daemon
        .start(std::time::Duration::from_secs(60 * 30))
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;
    Ok(GenericOk {
        ok: true,
        message: "daemon started".into(),
    })
}

async fn daemon_stop_inner(state: &AppState) -> Result<GenericOk, CommandError> {
    *state.daemon_should_run.write() = false;
    let daemon = state.daemon.clone();
    tauri::async_runtime::spawn(async move {
        daemon.stop().await;
    });
    Ok(GenericOk {
        ok: true,
        message: "daemon stop requested".into(),
    })
}

async fn daemon_restart_inner(state: &AppState) -> Result<GenericOk, CommandError> {
    *state.daemon_should_run.write() = true;
    let daemon = state.daemon.clone();
    tauri::async_runtime::spawn(async move {
        daemon.stop().await;
        let _ = daemon.start(std::time::Duration::from_secs(60 * 30)).await;
    });
    Ok(GenericOk {
        ok: true,
        message: "daemon restart requested".into(),
    })
}

async fn task_run_inner(state: &AppState, task_id: String) -> Result<GenericOk, CommandError> {
    if !state.daemon.list_tasks().iter().any(|t| t.id == task_id) {
        return Err(CommandError::validation(format!(
            "unknown task id: {task_id}"
        )));
    }
    push_suggestion(
        &state.suggestions,
        ProactiveSuggestion {
            at: Utc::now(),
            text: format!("Manual task run requested for '{task_id}'"),
        },
    );
    Ok(GenericOk {
        ok: true,
        message: "task run request accepted".into(),
    })
}

async fn config_show_inner(state: &AppState) -> Result<Value, CommandError> {
    Ok(serde_json::json!({
        "local_api_addr": state.local_api_addr,
        "mcp_configured": state.mcp.is_some(),
        "embedding_provider": if state.embedding_provider.is_some() { "provider" } else { "deterministic_fallback" },
        "allowed_roots": state.fs.allowed_roots(),
        "daemon_should_run": *state.daemon_should_run.read(),
    }))
}

async fn config_set_inner(
    _state: &AppState,
    key: String,
    _value: String,
) -> Result<GenericOk, CommandError> {
    Err(CommandError::validation(format!(
        "runtime config set is not supported for key '{key}'"
    )))
}

async fn operator_create_session_inner(
    state: &AppState,
    title: String,
    goal: String,
) -> Result<OperatorSession, CommandError> {
    let store = get_operator_store(state)?;
    store
        .create_session(&title, &goal)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

async fn operator_get_session_inner(
    state: &AppState,
    session_id: String,
) -> Result<OperatorSession, CommandError> {
    let store = get_operator_store(state)?;
    store
        .get_session(&session_id)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
        .ok_or_else(|| CommandError::validation(format!("session not found: {session_id}")))
}

async fn operator_list_sessions_inner(
    state: &AppState,
    limit: Option<usize>,
) -> Result<Vec<OperatorSession>, CommandError> {
    let store = get_operator_store(state)?;
    store
        .list_sessions(limit.unwrap_or(20))
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

async fn operator_plan_session_inner(
    state: &AppState,
    session_id: String,
    steps: Vec<String>,
) -> Result<Vec<OperatorStep>, CommandError> {
    let store = get_operator_store(state)?;
    if steps.is_empty() {
        return Err(CommandError::validation("steps must not be empty"));
    }
    store
        .replace_plan_steps(&session_id, &steps)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

async fn operator_run_session_inner(
    state: &AppState,
    session_id: String,
) -> Result<OperatorSession, CommandError> {
    let store = get_operator_store(state)?;
    let Some(_) = store
        .set_session_state(&session_id, OperatorSessionState::Running)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
    else {
        return Err(CommandError::validation(format!(
            "session not found: {session_id}"
        )));
    };

    let steps = store
        .list_steps(&session_id)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;
    if let Some(first) = steps.first() {
        let _ = store
            .propose_action(
                &session_id,
                Some(first.id.clone()),
                ActionKind::NetworkCall,
                Some("operator:run".into()),
                serde_json::json!({"step": first.title}),
                None,
                Some("operator run requested execution of planned step".into()),
            )
            .await
            .map_err(|e| CommandError::internal(e.to_string()))?;
        let _ = store
            .set_session_state(&session_id, OperatorSessionState::WaitingApproval)
            .await
            .map_err(|e| CommandError::internal(e.to_string()))?;
    } else {
        let _ = store
            .set_session_state(&session_id, OperatorSessionState::Completed)
            .await
            .map_err(|e| CommandError::internal(e.to_string()))?;
    }

    operator_get_session_inner(state, session_id).await
}

async fn operator_timeline_inner(
    state: &AppState,
    session_id: String,
    limit: Option<usize>,
) -> Result<Vec<OperatorTimelineItem>, CommandError> {
    let store = get_operator_store(state)?;
    store
        .timeline(&session_id, limit.unwrap_or(100))
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

async fn operator_pending_approvals_inner(
    state: &AppState,
    limit: Option<usize>,
) -> Result<Vec<OperatorActionProposal>, CommandError> {
    let store = get_operator_store(state)?;
    store
        .list_pending_approvals(limit.unwrap_or(100))
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

async fn operator_set_action_state_inner(
    state: &AppState,
    action_id: String,
    new_state: OperatorActionState,
    actor: Option<String>,
) -> Result<OperatorActionProposal, CommandError> {
    let store = get_operator_store(state)?;
    let actor = actor.unwrap_or_else(|| "desktop_ui".into());
    store
        .set_action_state(&action_id, new_state, &actor)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
        .ok_or_else(|| CommandError::validation(format!("action not found: {action_id}")))
}

async fn operator_create_task_inner(
    state: &AppState,
    req: OperatorTaskCreateReq,
) -> Result<OperatorTaskTemplate, CommandError> {
    let store = get_operator_store(state)?;
    store
        .create_task(NewOperatorTask {
            name: req.name,
            cron: req.cron,
            prompt: req.prompt,
            target_project: req.target_project,
            enabled: req.enabled.unwrap_or(true),
        })
        .await
        .map_err(|e| CommandError::validation(e.to_string()))
}

async fn operator_list_tasks_inner(
    state: &AppState,
    limit: Option<usize>,
) -> Result<Vec<OperatorTaskTemplate>, CommandError> {
    let store = get_operator_store(state)?;
    store
        .list_tasks(limit.unwrap_or(100))
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

async fn operator_run_task_now_inner(
    state: &AppState,
    task_id: String,
) -> Result<OperatorTaskTemplate, CommandError> {
    let store = get_operator_store(state)?;
    store
        .run_task_now(&task_id)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
        .ok_or_else(|| CommandError::validation(format!("task not found: {task_id}")))
}

async fn operator_list_projects_inner(
    state: &AppState,
    limit: Option<usize>,
) -> Result<Vec<ProjectInfo>, CommandError> {
    let store = get_operator_store(state)?;
    store
        .list_projects(limit.unwrap_or(100))
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

async fn operator_create_project_inner(
    state: &AppState,
    name: String,
    description: Option<String>,
) -> Result<ProjectInfo, CommandError> {
    let store = get_operator_store(state)?;
    store
        .create_project(&name, description)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

async fn operator_list_artifacts_inner(
    state: &AppState,
    project_id: String,
    limit: Option<usize>,
) -> Result<Vec<ProjectArtifact>, CommandError> {
    let store = get_operator_store(state)?;
    store
        .list_artifacts(&project_id, limit.unwrap_or(100))
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

async fn connectors_oauth_start_inner(
    state: &AppState,
    provider: String,
    req: ConnectorOAuthStartReq,
) -> Result<Value, CommandError> {
    let store = get_operator_store(state)?;
    let url = store
        .oauth_start(&provider, req.redirect_uri, req.scopes)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;
    Ok(serde_json::json!({
        "provider": provider,
        "auth_url": url
    }))
}

async fn connectors_oauth_callback_inner(
    state: &AppState,
    provider: String,
    req: ConnectorOAuthCallbackReq,
) -> Result<clawork_core::ConnectorStatus, CommandError> {
    let store = get_operator_store(state)?;
    store
        .oauth_callback(
            &provider,
            OAuthCallbackInput {
                account_id: req.account_id,
                access_token: req.access_token,
                refresh_token: req.refresh_token,
                scopes: req.scopes,
                error: req.error,
            },
        )
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

async fn connectors_status_inner(
    state: &AppState,
) -> Result<Vec<clawork_core::ConnectorStatus>, CommandError> {
    let store = get_operator_store(state)?;
    store
        .connector_statuses()
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

async fn research_create_job_inner(
    state: &AppState,
    req: ResearchJobCreateReq,
) -> Result<ResearchJobRecord, CommandError> {
    let project_id = req
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    let question = req.question.trim().to_string();
    if question.is_empty() {
        return Err(CommandError::validation("question is required"));
    }
    if req.source_urls.is_empty() {
        return Err(CommandError::validation("source_urls must not be empty"));
    }

    let mut source_urls = Vec::<String>::new();
    for raw in req.source_urls {
        let normalized = normalize_source_url(&raw);
        if normalized.is_empty() {
            continue;
        }
        if !normalized.starts_with("http://") && !normalized.starts_with("https://") {
            continue;
        }
        if !source_urls
            .iter()
            .any(|u| u.eq_ignore_ascii_case(&normalized))
        {
            source_urls.push(normalized);
        }
        if source_urls.len() >= 20 {
            break;
        }
    }
    if source_urls.is_empty() {
        return Err(CommandError::validation(
            "no valid http/https source_urls were provided",
        ));
    }

    let action = ActionRequest {
        kind: ActionKind::NetworkCall,
        target: source_urls
            .first()
            .cloned()
            .or_else(|| Some("research://job".into())),
        params: serde_json::json!({
            "question_len": question.len(),
            "sources": source_urls.len(),
        }),
        trace_id: Uuid::new_v4(),
    };

    let ctx = authorize_action(state, &action, req.approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();

    let store = get_operator_store(state)?;
    let title = req
        .title
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| truncate_text(&question, 80));
    let job = store
        .create_research_job(
            project_id.clone(),
            title,
            question.clone(),
            source_urls.clone(),
        )
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?;

    let report = build_research_report(state, &question, &source_urls).await;
    let mut artifact_id: Option<String> = None;
    let final_job = match report {
        Ok(report) => {
            let report_json = serde_json::to_value(report.clone())
                .map_err(|e| CommandError::internal(format!("serialize report failed: {e}")))?;
            if let Some(project_id) = project_id.as_deref() {
                artifact_id = Some(
                    register_research_report_artifact(
                        &store,
                        project_id,
                        &job.id,
                        &report_json,
                        &report.citations,
                        &source_urls,
                    )
                    .await?,
                );
            }
            store
                .complete_research_job(&job.id, report_json)
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?
                .ok_or_else(|| CommandError::internal("research job not found after completion"))?
        }
        Err(err) => store
            .fail_research_job(&job.id, err.clone())
            .await
            .map_err(|e| CommandError::internal(e.to_string()))?
            .ok_or_else(|| CommandError::internal("research job not found after failure"))?,
    };

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: if final_job.state == "completed" {
                "allowed".into()
            } else {
                "error".into()
            },
            reason: Some(format!(
                "mode={}, job_id={}, state={}, project_id={}, artifact_id={}",
                permission_mode_label(&mode),
                final_job.id,
                final_job.state,
                project_id.as_deref().unwrap_or("-"),
                artifact_id.as_deref().unwrap_or("-")
            )),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(final_job)
}

async fn register_research_report_artifact(
    store: &OperatorStore,
    project_id: &str,
    job_id: &str,
    report_json: &Value,
    citation_values: &[Value],
    source_urls: &[String],
) -> Result<String, CommandError> {
    let base = PathBuf::from("data").join("projects").join(project_id);
    tokio::fs::create_dir_all(&base)
        .await
        .map_err(|e| CommandError::internal(format!("create project artifact dir failed: {e}")))?;
    let output_path = base.join(format!("research-{job_id}.json"));
    let payload = serde_json::to_vec_pretty(report_json)
        .map_err(|e| CommandError::internal(format!("serialize report artifact failed: {e}")))?;
    tokio::fs::write(&output_path, payload)
        .await
        .map_err(|e| CommandError::internal(format!("write report artifact failed: {e}")))?;

    let citations = parse_report_citations(citation_values, source_urls);
    let artifact = store
        .add_artifact(
            project_id,
            output_path.display().to_string(),
            "application/json".to_string(),
            Some("research:report".to_string()),
            citations,
        )
        .await
        .map_err(|e| CommandError::internal(format!("artifact registration failed: {e}")))?;
    Ok(artifact.id)
}

async fn research_get_job_inner(
    state: &AppState,
    id: String,
) -> Result<ResearchJobRecord, CommandError> {
    let store = get_operator_store(state)?;
    store
        .get_research_job(&id)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
        .ok_or_else(|| CommandError::validation(format!("research job not found: {id}")))
}

async fn research_get_report_inner(state: &AppState, id: String) -> Result<Value, CommandError> {
    let job = research_get_job_inner(state, id).await?;
    if let Some(report) = job.report {
        return Ok(report);
    }
    if let Some(error) = job.error {
        return Err(CommandError::internal(format!(
            "research job failed: {error}"
        )));
    }
    Err(CommandError::validation(
        "research report is not available yet",
    ))
}

async fn media_generate_inner(
    state: &AppState,
    kind: &str,
    req: MediaGenerateReq,
) -> Result<MediaGenerateResult, CommandError> {
    let MediaGenerateReq {
        prompt: raw_prompt,
        provider,
        model,
        size,
        seconds,
        project_id,
        output_path,
        approval_token,
    } = req;

    let prompt = raw_prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(CommandError::validation("prompt is required"));
    }
    let provider = provider
        .unwrap_or_else(|| "default".to_string())
        .trim()
        .to_lowercase();
    let endpoint = media_provider_endpoint(kind, &provider).ok_or_else(|| {
        CommandError::not_configured(format!("media {kind} endpoint is not configured"))
    })?;

    let action = ActionRequest {
        kind: ActionKind::NetworkCall,
        target: Some(endpoint.clone()),
        params: serde_json::json!({
            "kind": kind,
            "provider": provider,
            "model": model.clone(),
            "size": size.clone(),
            "seconds": seconds
        }),
        trace_id: Uuid::new_v4(),
    };
    let ctx = authorize_action(state, &action, approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();

    let mut request = state.http.post(&endpoint).json(&serde_json::json!({
        "type": kind,
        "prompt": prompt,
        "model": model.clone(),
        "size": size.clone(),
        "seconds": seconds
    }));
    if let Some(api_key) = media_provider_api_key(&provider) {
        request = request.bearer_auth(api_key);
    }

    let response = request
        .send()
        .await
        .map_err(|e| CommandError::internal(format!("media provider request failed: {e}")))?;
    let status = response.status();
    let payload: Value = response.json().await.map_err(|e| {
        CommandError::internal(format!("media provider response parse failed: {e}"))
    })?;
    if !status.is_success() {
        return Err(CommandError::internal(format!(
            "media provider returned {}: {}",
            status.as_u16(),
            payload
        )));
    }

    let remote_url = extract_media_url(&payload);
    let Some(remote_url) = remote_url else {
        return Err(CommandError::validation(
            "media provider response must include url/output_url/result_url",
        ));
    };

    let binary = state
        .http
        .get(&remote_url)
        .send()
        .await
        .map_err(|e| CommandError::internal(format!("media download request failed: {e}")))?;
    if !binary.status().is_success() {
        return Err(CommandError::internal(format!(
            "media download failed with status {}",
            binary.status().as_u16()
        )));
    }

    let mime_type = binary
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or(match kind {
            "video" => "video/mp4",
            _ => "image/png",
        })
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    let bytes = binary
        .bytes()
        .await
        .map_err(|e| CommandError::internal(format!("media download read failed: {e}")))?;

    let output_path = if let Some(path) = output_path {
        PathBuf::from(path)
    } else {
        let ext = media_extension(&mime_type, kind);
        PathBuf::from("data").join("media").join(format!(
            "{kind}-{}.{}",
            Uuid::new_v4().simple(),
            ext
        ))
    };
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| CommandError::internal(format!("create media output dir failed: {e}")))?;
    }
    tokio::fs::write(&output_path, &bytes)
        .await
        .map_err(|e| CommandError::internal(format!("write media file failed: {e}")))?;

    let artifact_id = if let Some(project_id) = project_id {
        let store = get_operator_store(state)?;
        let artifact = store
            .add_artifact(
                &project_id,
                output_path.display().to_string(),
                mime_type.clone(),
                Some(format!("media:{kind}")),
                vec![CitationRef {
                    source_url: remote_url.clone(),
                    snippet_hash: snippet_sha256(&prompt),
                }],
            )
            .await
            .map_err(|e| CommandError::internal(format!("artifact registration failed: {e}")))?;
        Some(artifact.id)
    } else {
        None
    };

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: "allowed".into(),
            reason: Some(format!(
                "mode={}, kind={}, provider={}, output={}",
                permission_mode_label(&mode),
                kind,
                provider,
                output_path.display()
            )),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(MediaGenerateResult {
        ok: true,
        kind: kind.to_string(),
        provider,
        model,
        output_path: output_path.display().to_string(),
        mime_type,
        remote_url: Some(remote_url),
        artifact_id,
    })
}

async fn build_research_report(
    state: &AppState,
    question: &str,
    source_urls: &[String],
) -> Result<ResearchJobReport, String> {
    let mut citations = Vec::<Value>::new();
    let mut findings = Vec::<String>::new();
    let mut sources = Vec::<Value>::new();

    for url in source_urls {
        match fetch_research_source(state, url).await {
            Ok((snippet, title)) => {
                let snippet_hash = snippet_sha256(&snippet);
                let domain = extract_target_domain(url).unwrap_or_else(|| "source".into());
                findings.push(format!("{domain}: {}", truncate_text(&snippet, 220)));
                citations.push(serde_json::json!({
                    "source_url": url,
                    "snippet_hash": snippet_hash
                }));
                sources.push(serde_json::json!({
                    "source_url": url,
                    "title": title,
                    "snippet": snippet,
                    "ok": true
                }));
            }
            Err(error) => {
                sources.push(serde_json::json!({
                    "source_url": url,
                    "ok": false,
                    "error": error
                }));
            }
        }
    }

    let ok_count = sources
        .iter()
        .filter(|s| s.get("ok").and_then(Value::as_bool) == Some(true))
        .count();
    let summary = if ok_count == 0 {
        format!(
            "No sources could be fetched for this question: {}",
            truncate_text(question, 120)
        )
    } else {
        format!(
            "Collected {} sources ({} successful). Synthesized key findings with citations.",
            source_urls.len(),
            ok_count
        )
    };

    Ok(ResearchJobReport {
        generated_at: Utc::now(),
        question: question.to_string(),
        summary,
        findings,
        citations,
        sources,
    })
}

async fn connectors_google_sheets_append_inner(
    state: &AppState,
    req: GoogleSheetsAppendReq,
) -> Result<GoogleSheetsAppendResult, CommandError> {
    if req.spreadsheet_id.trim().is_empty() {
        return Err(CommandError::validation("spreadsheet_id is required"));
    }
    if req.values.is_empty() {
        return Err(CommandError::validation("values must not be empty"));
    }

    let sheet_name = req
        .sheet_name
        .unwrap_or_else(|| "Sheet1".to_string())
        .trim()
        .to_string();
    if sheet_name.is_empty() {
        return Err(CommandError::validation("sheet_name must not be empty"));
    }

    let value_input_option = req
        .value_input_option
        .unwrap_or_else(|| "RAW".to_string())
        .trim()
        .to_uppercase();
    if !matches!(value_input_option.as_str(), "RAW" | "USER_ENTERED") {
        return Err(CommandError::validation(
            "value_input_option must be RAW or USER_ENTERED",
        ));
    }

    let range = format!("{sheet_name}!A1");
    let encoded_range = encode_url_component(&range);
    let target = format!(
        "https://sheets.googleapis.com/v4/spreadsheets/{}/values/{}:append",
        req.spreadsheet_id, encoded_range
    );
    let action = ActionRequest {
        kind: ActionKind::NetworkCall,
        target: Some(target.clone()),
        params: serde_json::json!({
            "rows": req.values.len(),
            "sheet_name": sheet_name.clone(),
            "value_input_option": value_input_option.clone()
        }),
        trace_id: Uuid::new_v4(),
    };

    let ctx = authorize_action(state, &action, req.approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();
    let token = google_connector_access_token(state).await?;

    let resp = state
        .http
        .post(&target)
        .query(&[
            ("valueInputOption", value_input_option.as_str()),
            ("insertDataOption", "INSERT_ROWS"),
        ])
        .bearer_auth(token)
        .json(&serde_json::json!({
            "majorDimension": "ROWS",
            "values": req.values
        }))
        .send()
        .await
        .map_err(|e| CommandError::internal(format!("google sheets request failed: {e}")))?;

    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| CommandError::internal(format!("google sheets response parse failed: {e}")))?;
    if !status.is_success() {
        return Err(CommandError::internal(format!(
            "google sheets append failed ({}): {}",
            status.as_u16(),
            body
        )));
    }

    let updates = body.get("updates").cloned().unwrap_or(Value::Null);
    let updated_rows = updates.get("updatedRows").and_then(Value::as_i64);
    let updated_cells = updates.get("updatedCells").and_then(Value::as_i64);
    let updated_range = updates
        .get("updatedRange")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: "allowed".into(),
            reason: Some(format!(
                "mode={}, spreadsheet_id={}, updated_rows={}",
                permission_mode_label(&mode),
                req.spreadsheet_id,
                updated_rows.unwrap_or_default()
            )),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(GoogleSheetsAppendResult {
        ok: true,
        spreadsheet_id: req.spreadsheet_id,
        updated_range,
        updated_rows,
        updated_cells,
    })
}

async fn connectors_google_drive_create_inner(
    state: &AppState,
    req: GoogleDriveCreateReq,
) -> Result<GoogleDriveCreateResult, CommandError> {
    let GoogleDriveCreateReq {
        name,
        parent_id,
        mime_type,
        content,
        approval_token,
    } = req;

    if name.trim().is_empty() {
        return Err(CommandError::validation("name is required"));
    }
    if content.is_empty() {
        return Err(CommandError::validation("content must not be empty"));
    }

    let mime_type = mime_type
        .unwrap_or_else(|| "text/plain".to_string())
        .trim()
        .to_string();
    if mime_type.is_empty() {
        return Err(CommandError::validation("mime_type must not be empty"));
    }

    let action = ActionRequest {
        kind: ActionKind::NetworkCall,
        target: Some("https://www.googleapis.com/upload/drive/v3/files".into()),
        params: serde_json::json!({
            "name": name.clone(),
            "mime_type": mime_type.clone(),
            "parent_id": parent_id.clone()
        }),
        trace_id: Uuid::new_v4(),
    };

    let ctx = authorize_action(state, &action, approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();
    let token = google_connector_access_token(state).await?;

    let mut metadata = serde_json::json!({
        "name": name,
        "mimeType": mime_type
    });
    if let Some(parent_id) = parent_id {
        metadata["parents"] = serde_json::json!([parent_id]);
    }

    let boundary = format!("clawork-{}", Uuid::new_v4().simple());
    let mut body = Vec::<u8>::new();
    write!(
        body,
        "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n"
    )
    .map_err(|e| CommandError::internal(format!("multipart metadata build failed: {e}")))?;
    write!(body, "--{boundary}\r\nContent-Type: {mime_type}\r\n\r\n")
        .map_err(|e| CommandError::internal(format!("multipart media header build failed: {e}")))?;
    body.extend_from_slice(content.as_bytes());
    write!(body, "\r\n--{boundary}--\r\n")
        .map_err(|e| CommandError::internal(format!("multipart closing build failed: {e}")))?;

    let resp = state
        .http
        .post("https://www.googleapis.com/upload/drive/v3/files")
        .query(&[
            ("uploadType", "multipart"),
            ("fields", "id,name,mimeType,webViewLink,webContentLink"),
        ])
        .header(
            "Content-Type",
            format!("multipart/related; boundary={boundary}"),
        )
        .bearer_auth(token)
        .body(body)
        .send()
        .await
        .map_err(|e| CommandError::internal(format!("google drive request failed: {e}")))?;

    let status = resp.status();
    let payload: Value = resp
        .json()
        .await
        .map_err(|e| CommandError::internal(format!("google drive response parse failed: {e}")))?;
    if !status.is_success() {
        return Err(CommandError::internal(format!(
            "google drive create failed ({}): {}",
            status.as_u16(),
            payload
        )));
    }

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: "allowed".into(),
            reason: Some(format!("mode={}", permission_mode_label(&mode))),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(GoogleDriveCreateResult {
        ok: true,
        id: payload
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        name: payload
            .get("name")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        mime_type: payload
            .get("mimeType")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        web_view_link: payload
            .get("webViewLink")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        web_content_link: payload
            .get("webContentLink")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

async fn connectors_notion_search_inner(
    state: &AppState,
    req: NotionSearchReq,
) -> Result<NotionSearchResult, CommandError> {
    let query = req.query.trim().to_string();
    if query.is_empty() {
        return Err(CommandError::validation("query is required"));
    }
    let page_size = req.page_size.unwrap_or(10).clamp(1, 100);

    let action = ActionRequest {
        kind: ActionKind::NetworkCall,
        target: Some("https://api.notion.com/v1/search".into()),
        params: serde_json::json!({
            "query_len": query.len(),
            "page_size": page_size
        }),
        trace_id: Uuid::new_v4(),
    };
    let ctx = authorize_action(state, &action, req.approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();
    let token = notion_connector_access_token(state).await?;
    let notion_version = std::env::var("CLAWORK_NOTION_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "2022-06-28".to_string());

    let resp = state
        .http
        .post("https://api.notion.com/v1/search")
        .bearer_auth(token)
        .header("Notion-Version", notion_version)
        .json(&serde_json::json!({
            "query": query,
            "page_size": page_size
        }))
        .send()
        .await
        .map_err(|e| CommandError::internal(format!("notion search request failed: {e}")))?;

    let status = resp.status();
    let payload: Value = resp
        .json()
        .await
        .map_err(|e| CommandError::internal(format!("notion search response parse failed: {e}")))?;
    if !status.is_success() {
        return Err(CommandError::internal(format!(
            "notion search failed ({}): {}",
            status.as_u16(),
            payload
        )));
    }

    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: "allowed".into(),
            reason: Some(format!(
                "mode={}, provider=notion, query_len={}, hits={}",
                permission_mode_label(&mode),
                query.len(),
                results.len()
            )),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(NotionSearchResult {
        ok: true,
        query,
        total: results.len(),
        results,
    })
}

async fn connectors_notion_page_create_inner(
    state: &AppState,
    req: NotionPageCreateReq,
) -> Result<NotionPageCreateResult, CommandError> {
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(CommandError::validation("title is required"));
    }

    let parent_page_id = req
        .parent_page_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    let parent_database_id = req
        .parent_database_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    if parent_page_id.is_some() && parent_database_id.is_some() {
        return Err(CommandError::validation(
            "use either parent_page_id or parent_database_id",
        ));
    }

    let content = req
        .content
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    let action = ActionRequest {
        kind: ActionKind::NetworkCall,
        target: Some("https://api.notion.com/v1/pages".into()),
        params: serde_json::json!({
            "title_len": title.len(),
            "parent_page_id": parent_page_id.clone(),
            "parent_database_id": parent_database_id.clone()
        }),
        trace_id: Uuid::new_v4(),
    };
    let ctx = authorize_action(state, &action, req.approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();
    let token = notion_connector_access_token(state).await?;
    let notion_version = std::env::var("CLAWORK_NOTION_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "2022-06-28".to_string());

    let parent = if let Some(page_id) = parent_page_id {
        serde_json::json!({ "page_id": page_id })
    } else if let Some(database_id) = parent_database_id {
        serde_json::json!({ "database_id": database_id })
    } else {
        serde_json::json!({ "workspace": true })
    };

    let mut title_property_name = "title".to_string();
    if let Some(database_id) = parent
        .get("database_id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
    {
        let db_resp = state
            .http
            .get(format!("https://api.notion.com/v1/databases/{database_id}"))
            .bearer_auth(&token)
            .header("Notion-Version", &notion_version)
            .send()
            .await
            .map_err(|e| {
                CommandError::internal(format!("notion database schema request failed: {e}"))
            })?;
        let db_status = db_resp.status();
        let db_payload: Value = db_resp.json().await.map_err(|e| {
            CommandError::internal(format!("notion database schema parse failed: {e}"))
        })?;
        if !db_status.is_success() {
            return Err(CommandError::internal(format!(
                "notion database schema request failed ({}): {}",
                db_status.as_u16(),
                db_payload
            )));
        }
        if let Some(props) = db_payload.get("properties").and_then(Value::as_object) {
            if let Some((name, _)) = props
                .iter()
                .find(|(_, v)| v.get("type").and_then(Value::as_str) == Some("title"))
            {
                title_property_name = name.clone();
            }
        }
    }

    let mut properties = serde_json::Map::new();
    properties.insert(
        title_property_name,
        serde_json::json!({
            "title": [{
                "text": { "content": title }
            }]
        }),
    );
    let mut body = serde_json::json!({
        "parent": parent,
        "properties": Value::Object(properties)
    });
    if let Some(content) = content {
        body["children"] = serde_json::json!([{
            "object": "block",
            "type": "paragraph",
            "paragraph": {
                "rich_text": [{
                    "type": "text",
                    "text": { "content": content }
                }]
            }
        }]);
    }

    let resp = state
        .http
        .post("https://api.notion.com/v1/pages")
        .bearer_auth(token)
        .header("Notion-Version", notion_version)
        .json(&body)
        .send()
        .await
        .map_err(|e| CommandError::internal(format!("notion page create request failed: {e}")))?;

    let status = resp.status();
    let payload: Value = resp
        .json()
        .await
        .map_err(|e| CommandError::internal(format!("notion page create parse failed: {e}")))?;
    if !status.is_success() {
        return Err(CommandError::internal(format!(
            "notion page create failed ({}): {}",
            status.as_u16(),
            payload
        )));
    }

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: "allowed".into(),
            reason: Some(format!(
                "mode={}, provider=notion",
                permission_mode_label(&mode)
            )),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(NotionPageCreateResult {
        ok: true,
        id: payload
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        url: payload
            .get("url")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        created_time: payload
            .get("created_time")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

async fn connectors_slack_post_inner(
    state: &AppState,
    req: SlackPostReq,
) -> Result<SlackPostResult, CommandError> {
    let channel = req.channel.trim().to_string();
    let text = req.text.trim().to_string();
    if channel.is_empty() {
        return Err(CommandError::validation("channel is required"));
    }
    if text.is_empty() {
        return Err(CommandError::validation("text is required"));
    }

    let action = ActionRequest {
        kind: ActionKind::NetworkCall,
        target: Some("https://slack.com/api/chat.postMessage".into()),
        params: serde_json::json!({
            "channel": channel.clone(),
            "text_len": text.len()
        }),
        trace_id: Uuid::new_v4(),
    };
    let ctx = authorize_action(state, &action, req.approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();
    let token = slack_connector_access_token(state).await?;

    let resp = state
        .http
        .post("https://slack.com/api/chat.postMessage")
        .bearer_auth(token)
        .json(&serde_json::json!({
            "channel": channel,
            "text": text
        }))
        .send()
        .await
        .map_err(|e| CommandError::internal(format!("slack post request failed: {e}")))?;
    let status = resp.status();
    let payload: Value = resp
        .json()
        .await
        .map_err(|e| CommandError::internal(format!("slack post parse failed: {e}")))?;
    if !status.is_success() {
        return Err(CommandError::internal(format!(
            "slack post failed ({}): {}",
            status.as_u16(),
            payload
        )));
    }
    if payload.get("ok").and_then(Value::as_bool) != Some(true) {
        let error = payload
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown_error");
        return Err(CommandError::internal(format!(
            "slack post failed: {error}"
        )));
    }

    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: "allowed".into(),
            reason: Some(format!(
                "mode={}, provider=slack",
                permission_mode_label(&mode)
            )),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(SlackPostResult {
        ok: true,
        channel: payload
            .get("channel")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        ts: payload
            .get("ts")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        text: payload
            .get("message")
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

async fn connectors_slack_history_inner(
    state: &AppState,
    req: SlackHistoryReq,
) -> Result<SlackHistoryResult, CommandError> {
    let channel = req.channel.trim().to_string();
    if channel.is_empty() {
        return Err(CommandError::validation("channel is required"));
    }
    let limit = req.limit.unwrap_or(20).clamp(1, 100);

    let action = ActionRequest {
        kind: ActionKind::NetworkCall,
        target: Some("https://slack.com/api/conversations.history".into()),
        params: serde_json::json!({
            "channel": channel.clone(),
            "limit": limit
        }),
        trace_id: Uuid::new_v4(),
    };
    let ctx = authorize_action(state, &action, req.approval_token).await?;
    ensure_action_still_authorized(state, &action, &ctx).await?;
    let mode = state.permissions.current_mode();
    let token = slack_connector_access_token(state).await?;
    let limit_str = limit.to_string();

    let resp = state
        .http
        .get("https://slack.com/api/conversations.history")
        .bearer_auth(token)
        .query(&[("channel", channel.as_str()), ("limit", limit_str.as_str())])
        .send()
        .await
        .map_err(|e| CommandError::internal(format!("slack history request failed: {e}")))?;
    let status = resp.status();
    let payload: Value = resp
        .json()
        .await
        .map_err(|e| CommandError::internal(format!("slack history parse failed: {e}")))?;
    if !status.is_success() {
        return Err(CommandError::internal(format!(
            "slack history failed ({}): {}",
            status.as_u16(),
            payload
        )));
    }
    if payload.get("ok").and_then(Value::as_bool) != Some(true) {
        let error = payload
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown_error");
        return Err(CommandError::internal(format!(
            "slack history failed: {error}"
        )));
    }

    let messages = payload
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    append_audit(
        state,
        &AuditEvent {
            timestamp: Utc::now(),
            actor: ctx.actor,
            action: action.kind,
            target: action.target,
            decision: "allowed".into(),
            reason: Some(format!(
                "mode={}, provider=slack, channel={}, count={}",
                permission_mode_label(&mode),
                channel,
                messages.len()
            )),
            trace_id: action.trace_id,
        },
    )
    .await;

    Ok(SlackHistoryResult {
        ok: true,
        channel,
        messages,
    })
}

async fn policy_set_domain_inner(
    state: &AppState,
    req: DomainPolicySetReq,
) -> Result<DomainPolicy, CommandError> {
    let store = get_operator_store(state)?;
    store
        .set_domain_policy(
            req.domain,
            req.profile,
            req.blocked_actions,
            req.allow_actions,
        )
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

async fn policy_list_domain_inner(state: &AppState) -> Result<Vec<DomainPolicy>, CommandError> {
    let store = get_operator_store(state)?;
    store
        .list_domain_policies()
        .await
        .map_err(|e| CommandError::internal(e.to_string()))
}

fn telegram_update_to_messages(update: TelegramWebhookUpdate) -> Vec<InboundMessage> {
    let Some(message) = update.message else {
        return vec![];
    };

    let content = message
        .text
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "[non-text message]".to_string());

    vec![InboundMessage {
        adapter: "telegram".into(),
        from: message.chat.id.to_string(),
        content,
        received_at: Utc::now(),
        external_id: Some(update.update_id.to_string()),
    }]
}

fn whatsapp_payload_to_messages(payload: &Value) -> Vec<InboundMessage> {
    let mut inbound = Vec::new();
    let Some(entries) = payload.get("entry").and_then(Value::as_array) else {
        return inbound;
    };

    for entry in entries {
        let Some(changes) = entry.get("changes").and_then(Value::as_array) else {
            continue;
        };
        for change in changes {
            let Some(value) = change.get("value") else {
                continue;
            };
            let Some(messages) = value.get("messages").and_then(Value::as_array) else {
                continue;
            };
            for msg in messages {
                let Some(from) = msg.get("from").and_then(Value::as_str) else {
                    continue;
                };

                let msg_type = msg.get("type").and_then(Value::as_str).unwrap_or("text");
                let content = match msg_type {
                    "text" => msg
                        .get("text")
                        .and_then(|v| v.get("body"))
                        .and_then(Value::as_str)
                        .unwrap_or("[empty text]")
                        .to_string(),
                    other => format!("[{other} message]"),
                };
                let external_id = msg
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);

                inbound.push(InboundMessage {
                    adapter: "whatsapp".into(),
                    from: from.to_string(),
                    content,
                    received_at: Utc::now(),
                    external_id,
                });
            }
        }
    }

    inbound
}

fn verify_telegram_webhook(headers: &HeaderMap) -> Result<(), CommandError> {
    let expected = std::env::var("CLAWORK_TELEGRAM_WEBHOOK_SECRET").unwrap_or_default();
    if expected.trim().is_empty() {
        return Ok(());
    }

    let provided = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if provided == expected {
        Ok(())
    } else {
        Err(CommandError::denied("invalid telegram webhook secret"))
    }
}

fn verify_whatsapp_webhook_signature(
    headers: &HeaderMap,
    raw_body: &[u8],
) -> Result<(), CommandError> {
    let app_secret = std::env::var("CLAWORK_WHATSAPP_APP_SECRET").map_err(|_| {
        CommandError::not_configured(
            "CLAWORK_WHATSAPP_APP_SECRET is required for webhook verification",
        )
    })?;
    if app_secret.trim().is_empty() {
        return Err(CommandError::not_configured(
            "CLAWORK_WHATSAPP_APP_SECRET is empty",
        ));
    }

    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| CommandError::denied("missing x-hub-signature-256"))?;
    let hex_signature = signature
        .strip_prefix("sha256=")
        .ok_or_else(|| CommandError::denied("invalid signature format"))?;
    let expected_bytes =
        hex::decode(hex_signature).map_err(|_| CommandError::denied("invalid signature hex"))?;

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(app_secret.as_bytes())
        .map_err(|_| CommandError::internal("failed to initialize signature verifier"))?;
    mac.update(raw_body);
    mac.verify_slice(&expected_bytes)
        .map_err(|_| CommandError::denied("invalid whatsapp webhook signature"))?;
    Ok(())
}

async fn run_local_api(state: Arc<AppState>) -> anyhow::Result<()> {
    let router = Router::new()
        .route("/v1/status", get(api_get_status))
        .route("/v1/tasks", get(api_get_tasks))
        .route(
            "/v1/operator/sessions",
            post(api_operator_create_session).get(api_operator_list_sessions),
        )
        .route("/v1/operator/sessions/{id}", get(api_operator_get_session))
        .route(
            "/v1/operator/sessions/{id}/plan",
            post(api_operator_plan_session),
        )
        .route(
            "/v1/operator/sessions/{id}/run",
            post(api_operator_run_session),
        )
        .route(
            "/v1/operator/sessions/{id}/timeline",
            get(api_operator_timeline),
        )
        .route(
            "/v1/operator/actions/{id}/approve",
            post(api_operator_action_approve),
        )
        .route(
            "/v1/operator/actions/{id}/reject",
            post(api_operator_action_reject),
        )
        .route(
            "/v1/operator/approvals/pending",
            get(api_operator_pending_approvals),
        )
        .route(
            "/v1/operator/tasks",
            post(api_operator_create_task).get(api_operator_list_tasks),
        )
        .route(
            "/v1/operator/tasks/{id}/run-now",
            post(api_operator_run_task_now),
        )
        .route(
            "/v1/operator/projects",
            post(api_operator_create_project).get(api_operator_list_projects),
        )
        .route(
            "/v1/operator/projects/{id}/artifacts",
            get(api_operator_list_artifacts),
        )
        .route(
            "/v1/connectors/{provider}/oauth/start",
            post(api_connectors_oauth_start),
        )
        .route(
            "/v1/connectors/{provider}/oauth/callback",
            post(api_connectors_oauth_callback),
        )
        .route("/v1/connectors/status", get(api_connectors_status))
        .route(
            "/v1/connectors/google/sheets/append",
            post(api_connectors_google_sheets_append),
        )
        .route(
            "/v1/connectors/google/drive/create",
            post(api_connectors_google_drive_create),
        )
        .route(
            "/v1/connectors/notion/search",
            post(api_connectors_notion_search),
        )
        .route(
            "/v1/connectors/notion/page/create",
            post(api_connectors_notion_page_create),
        )
        .route("/v1/connectors/slack/post", post(api_connectors_slack_post))
        .route(
            "/v1/connectors/slack/history",
            post(api_connectors_slack_history),
        )
        .route(
            "/v1/policies/domain",
            post(api_policy_set_domain).get(api_policy_list_domain),
        )
        .route("/v1/research/jobs", post(api_research_create_job))
        .route("/v1/research/jobs/{id}", get(api_research_get_job))
        .route(
            "/v1/research/jobs/{id}/report",
            get(api_research_get_report),
        )
        .route("/v1/media/image", post(api_media_image))
        .route("/v1/media/video", post(api_media_video))
        .route("/v1/skills/list", get(api_list_skills))
        .route("/v1/actions/approve", post(api_approve_action))
        .route("/v1/auth/token/issue", post(api_auth_token_issue))
        .route("/v1/auth/token/revoke", post(api_auth_token_revoke))
        .route("/v1/auth/token/rotate", post(api_auth_token_rotate))
        .route("/v1/skills/run", post(api_run_skill))
        .route("/v1/skills/create", post(api_create_skill))
        .route("/v1/messages/send", post(api_send_message))
        .route("/v1/nl/execute", post(api_nl_execute))
        .route("/v1/messages/inbound", get(api_inbound_messages))
        .route("/v1/mail/inbox/unreplied", get(api_mail_inbox_unreplied))
        .route("/v1/fs/operate", post(api_fs_operate))
        .route("/v1/mcp/call", post(api_mcp_call))
        .route("/v1/browser/navigate", post(api_browser_navigate))
        .route("/v1/office/excel", post(api_office_excel))
        .route("/v1/office/upload", post(api_office_upload))
        .route("/v1/memory/store", post(api_memory_store))
        .route("/v1/memory/recent", get(api_memory_recent))
        .route("/v1/memory/search", post(api_memory_search))
        .route("/v1/briefing", get(api_briefing))
        .route("/v1/suggestions", get(api_suggestions))
        .route("/v1/daemon/start", post(api_daemon_start))
        .route("/v1/daemon/stop", post(api_daemon_stop))
        .route("/v1/daemon/restart", post(api_daemon_restart))
        .route("/v1/tasks/run", post(api_task_run))
        .route("/v1/logs", get(api_logs))
        .route("/v1/config", get(api_config_show))
        .route("/v1/config/set", post(api_config_set))
        .route("/v1/webhooks/telegram", post(api_telegram_webhook))
        .route(
            "/v1/webhooks/whatsapp",
            get(api_whatsapp_webhook_verify).post(api_whatsapp_webhook_event),
        )
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(&state.local_api_addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

async fn authorize_action(
    state: &AppState,
    action: &ActionRequest,
    approval_token: Option<String>,
) -> Result<RequestContext, CommandError> {
    audit_elevation_expiry_if_needed(state, "desktop_ui", action.trace_id).await;

    if let Some(target) = &action.target {
        if let Some(domain) = extract_target_domain(target) {
            if let Ok(store) = get_operator_store(state) {
                if let Ok(Some(policy)) = store.get_domain_policy(&domain).await {
                    if policy.blocked_actions.contains(&action.kind) {
                        append_audit(
                            state,
                            &AuditEvent {
                                timestamp: Utc::now(),
                                actor: "desktop_ui".into(),
                                action: action.kind.clone(),
                                target: action.target.clone(),
                                decision: "denied".into(),
                                reason: Some(format!(
                                    "domain policy blocked action kind for domain '{domain}'"
                                )),
                                trace_id: action.trace_id,
                            },
                        )
                        .await;
                        return Err(CommandError::denied(format!(
                            "action blocked by domain policy for '{domain}'"
                        )));
                    }
                }
            }
        }
    }

    let ctx = RequestContext {
        actor: "desktop_ui".into(),
        mode: state.permissions.current_mode(),
        approval_token,
    };

    match state.permissions.authorize(action, &ctx).await {
        Decision::Allow => Ok(ctx),
        Decision::Deny { reason } => {
            append_audit(
                state,
                &AuditEvent {
                    timestamp: Utc::now(),
                    actor: ctx.actor,
                    action: action.kind.clone(),
                    target: action.target.clone(),
                    decision: "denied".into(),
                    reason: Some(reason.clone()),
                    trace_id: action.trace_id,
                },
            )
            .await;
            Err(CommandError::denied(reason))
        }
        Decision::RequiresConfirmation { token, reason } => {
            append_audit(
                state,
                &AuditEvent {
                    timestamp: Utc::now(),
                    actor: ctx.actor,
                    action: action.kind.clone(),
                    target: action.target.clone(),
                    decision: "confirmation_required".into(),
                    reason: Some(reason.clone()),
                    trace_id: action.trace_id,
                },
            )
            .await;
            Err(CommandError::confirmation(token, reason))
        }
    }
}

async fn ensure_action_still_authorized(
    state: &AppState,
    action: &ActionRequest,
    ctx: &RequestContext,
) -> Result<(), CommandError> {
    audit_elevation_expiry_if_needed(state, &ctx.actor, action.trace_id).await;

    let recheck_ctx = RequestContext {
        actor: ctx.actor.clone(),
        mode: state.permissions.current_mode(),
        approval_token: ctx.approval_token.clone(),
    };

    match state.permissions.authorize(action, &recheck_ctx).await {
        Decision::Allow => Ok(()),
        Decision::Deny { reason } => {
            append_audit(
                state,
                &AuditEvent {
                    timestamp: Utc::now(),
                    actor: ctx.actor.clone(),
                    action: action.kind.clone(),
                    target: action.target.clone(),
                    decision: "denied".into(),
                    reason: Some(format!("revalidation_failed: {reason}")),
                    trace_id: action.trace_id,
                },
            )
            .await;
            Err(CommandError::denied(reason))
        }
        Decision::RequiresConfirmation { token, reason } => {
            append_audit(
                state,
                &AuditEvent {
                    timestamp: Utc::now(),
                    actor: ctx.actor.clone(),
                    action: action.kind.clone(),
                    target: action.target.clone(),
                    decision: "confirmation_required".into(),
                    reason: Some(format!("revalidation_required: {reason}")),
                    trace_id: action.trace_id,
                },
            )
            .await;
            Err(CommandError::confirmation(token, reason))
        }
    }
}

async fn audit_elevation_expiry_if_needed(state: &AppState, actor: &str, trace_id: Uuid) {
    if let Some(expired_at) = state.permissions.expire_elevation_if_needed() {
        append_audit(
            state,
            &AuditEvent {
                timestamp: Utc::now(),
                actor: actor.to_string(),
                action: ActionKind::ElevatedRequest,
                target: Some("local-session".into()),
                decision: "expired".into(),
                reason: Some(format!(
                    "mode_transition=elevated->sandbox, expires_at={expired_at}"
                )),
                trace_id,
            },
        )
        .await;
    }
}

async fn append_audit(state: &AppState, event: &AuditEvent) {
    let store = {
        let guard = state.audit.read();
        guard.clone()
    };
    if let Some(store) = store {
        let _ = store.append(event).await;
    }
}

fn get_memory_store(state: &AppState) -> Result<MemoryStore, CommandError> {
    let store = {
        let guard = state.memory.read();
        guard.clone()
    };
    store.ok_or_else(|| CommandError::not_configured("memory store not initialized"))
}

fn get_operator_store(state: &AppState) -> Result<OperatorStore, CommandError> {
    let store = {
        let guard = state.operator.read();
        guard.clone()
    };
    store.ok_or_else(|| CommandError::not_configured("operator store not initialized"))
}

async fn connector_access_token_from_env_or_store(
    state: &AppState,
    provider: &str,
    env_name: &str,
) -> Result<String, CommandError> {
    if let Ok(token) = std::env::var(env_name) {
        let trimmed = token.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let store = get_operator_store(state)?;
    if let Some(token) = store
        .connector_access_token(provider)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
    {
        return Ok(token);
    }

    if let Some(status) = store
        .connector_status(provider)
        .await
        .map_err(|e| CommandError::internal(e.to_string()))?
    {
        if !status.connected {
            let detail = status
                .last_error
                .map(|e| format!(" last_error={e}"))
                .unwrap_or_default();
            return Err(CommandError::not_configured(format!(
                "{provider} connector is not connected.{detail}"
            )));
        }
    }

    Err(CommandError::not_configured(format!(
        "{provider} connector access token is missing; complete oauth callback first"
    )))
}

async fn google_connector_access_token(state: &AppState) -> Result<String, CommandError> {
    connector_access_token_from_env_or_store(state, "google", "CLAWORK_GOOGLE_ACCESS_TOKEN").await
}

async fn notion_connector_access_token(state: &AppState) -> Result<String, CommandError> {
    connector_access_token_from_env_or_store(state, "notion", "CLAWORK_NOTION_ACCESS_TOKEN").await
}

async fn slack_connector_access_token(state: &AppState) -> Result<String, CommandError> {
    connector_access_token_from_env_or_store(state, "slack", "CLAWORK_SLACK_BOT_TOKEN").await
}

fn encode_url_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

fn find_gmail_header(headers: &[Value], name: &str) -> Option<String> {
    headers.iter().find_map(|header| {
        let key = header.get("name").and_then(Value::as_str)?;
        if key.eq_ignore_ascii_case(name) {
            header
                .get("value")
                .and_then(Value::as_str)
                .map(|v| v.trim().to_string())
        } else {
            None
        }
    })
}

fn build_reply_suggestion(from: &str, subject: &str, snippet: &str) -> String {
    let preview = snippet
        .split_whitespace()
        .take(20)
        .collect::<Vec<_>>()
        .join(" ");
    if preview.is_empty() {
        format!("Hi {from}, thanks for your email about \"{subject}\". I will review and reply shortly.")
    } else {
        format!("Hi {from}, thanks for your email about \"{subject}\". I reviewed your note (\"{preview}\"). I will follow up shortly.")
    }
}

async fn fetch_research_source(
    state: &AppState,
    source_url: &str,
) -> Result<(String, Option<String>), String> {
    let resp = state
        .http
        .get(source_url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("http status {}", status.as_u16()));
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("read body failed: {e}"))?;

    let (title, text) = if content_type.contains("text/html") || body.contains("<html") {
        (
            extract_html_title(&body),
            normalize_whitespace(&strip_html_tags(&body)),
        )
    } else {
        (None, normalize_whitespace(&body))
    };
    let snippet = truncate_text(&text, 1200);
    if snippet.trim().is_empty() {
        return Err("empty source content".into());
    }
    Ok((snippet, title))
}

fn strip_html_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ => {
                if !in_tag {
                    out.push(ch);
                }
            }
        }
    }
    out
}

fn extract_html_title(input: &str) -> Option<String> {
    let lower = input.to_ascii_lowercase();
    let title_start = lower.find("<title")?;
    let start_tag_end = lower[title_start..].find('>')? + title_start + 1;
    let title_end = lower[start_tag_end..].find("</title>")? + start_tag_end;
    let raw = input[start_tag_end..title_end].trim();
    if raw.is_empty() {
        None
    } else {
        Some(normalize_whitespace(raw))
    }
}

fn normalize_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_text(input: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in input.chars().take(max_chars.max(1)) {
        out.push(ch);
    }
    out
}

fn snippet_sha256(snippet: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(snippet.as_bytes());
    hex::encode(hasher.finalize())
}

fn normalize_source_url(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    if let Some((left, _)) = s.split_once('#') {
        s = left.to_string();
    }
    while s.ends_with('/') {
        s.pop();
    }
    s
}

fn parse_report_citations(citations: &[Value], source_urls: &[String]) -> Vec<CitationRef> {
    let mut out = Vec::<CitationRef>::new();
    for value in citations {
        let Some(source_url) = value.get("source_url").and_then(Value::as_str) else {
            continue;
        };
        let source_url = normalize_source_url(source_url);
        if source_url.is_empty() {
            continue;
        }
        let snippet_hash = value
            .get("snippet_hash")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| snippet_sha256(&source_url));
        if out
            .iter()
            .any(|existing| existing.source_url.eq_ignore_ascii_case(&source_url))
        {
            continue;
        }
        out.push(CitationRef {
            source_url,
            snippet_hash,
        });
    }

    if out.is_empty() {
        for source_url in source_urls {
            let normalized = normalize_source_url(source_url);
            if normalized.is_empty() {
                continue;
            }
            if out
                .iter()
                .any(|existing| existing.source_url.eq_ignore_ascii_case(&normalized))
            {
                continue;
            }
            out.push(CitationRef {
                source_url: normalized.clone(),
                snippet_hash: snippet_sha256(&normalized),
            });
        }
    }

    out
}

fn media_provider_endpoint(kind: &str, provider: &str) -> Option<String> {
    let provider_upper = provider.replace('-', "_").to_ascii_uppercase();
    let kind_upper = kind.to_ascii_uppercase();
    let specific_key = format!("CLAWORK_MEDIA_ENDPOINT_{provider_upper}_{kind_upper}");
    if let Ok(v) = std::env::var(&specific_key) {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let generic_key = format!("CLAWORK_MEDIA_{kind_upper}_ENDPOINT");
    if let Ok(v) = std::env::var(&generic_key) {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}

fn media_provider_api_key(provider: &str) -> Option<String> {
    let provider_upper = provider.replace('-', "_").to_ascii_uppercase();
    let specific_key = format!("CLAWORK_MEDIA_API_KEY_{provider_upper}");
    if let Ok(v) = std::env::var(&specific_key) {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Ok(v) = std::env::var("CLAWORK_MEDIA_API_KEY") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn extract_media_url(payload: &Value) -> Option<String> {
    if let Some(url) = payload.get("url").and_then(Value::as_str) {
        return Some(url.to_string());
    }
    if let Some(url) = payload.get("output_url").and_then(Value::as_str) {
        return Some(url.to_string());
    }
    if let Some(url) = payload.get("result_url").and_then(Value::as_str) {
        return Some(url.to_string());
    }
    if let Some(url) = payload.get("image_url").and_then(Value::as_str) {
        return Some(url.to_string());
    }
    if let Some(url) = payload.get("video_url").and_then(Value::as_str) {
        return Some(url.to_string());
    }
    if let Some(url) = payload
        .get("data")
        .and_then(|v| v.get("url"))
        .and_then(Value::as_str)
    {
        return Some(url.to_string());
    }
    None
}

fn media_extension(mime_type: &str, kind: &str) -> &'static str {
    let mime = mime_type.to_ascii_lowercase();
    if mime.contains("jpeg") || mime.contains("jpg") {
        "jpg"
    } else if mime.contains("webp") {
        "webp"
    } else if mime.contains("gif") {
        "gif"
    } else if mime.contains("mp4") {
        "mp4"
    } else if mime.contains("webm") {
        "webm"
    } else if kind == "video" {
        "mp4"
    } else {
        "png"
    }
}

fn permission_mode_label(mode: &PermissionMode) -> String {
    match mode {
        PermissionMode::Sandbox => "sandbox".into(),
        PermissionMode::Elevated { expires_at } => format!("elevated(until={expires_at})"),
    }
}

fn extract_target_domain(target: &str) -> Option<String> {
    if let Some(after_scheme) = target.split("://").nth(1) {
        let host = after_scheme
            .split('/')
            .next()
            .unwrap_or_default()
            .split(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if !host.is_empty() {
            return Some(host);
        }
    }

    let plain = target.trim().to_lowercase();
    if plain.contains('.') && !plain.contains('\\') && !plain.contains(' ') {
        return Some(
            plain
                .split('/')
                .next()
                .unwrap_or_default()
                .split(':')
                .next()
                .unwrap_or_default()
                .to_string(),
        );
    }
    None
}

async fn embedding_for_text(state: &AppState, text: &str) -> Vec<f32> {
    if let Some(provider) = &state.embedding_provider {
        let req = vec![text.to_string()];
        if let Ok(vectors) = provider.embed(&req).await {
            if let Some(first) = vectors.into_iter().next() {
                if !first.is_empty() {
                    return first;
                }
            }
        }
    }

    deterministic_embedding(text)
}

fn deterministic_embedding(text: &str) -> Vec<f32> {
    const DIM: usize = 128;
    let mut vec = vec![0.0f32; DIM];
    for (i, b) in text.as_bytes().iter().enumerate() {
        let idx = (i * 131 + (*b as usize)) % DIM;
        vec[idx] += (*b as f32) / 255.0;
    }
    let norm = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for x in &mut vec {
            *x /= norm;
        }
    }
    vec
}

fn push_suggestion(store: &Arc<RwLock<Vec<ProactiveSuggestion>>>, suggestion: ProactiveSuggestion) {
    let mut list = store.write();
    list.push(suggestion);
    if list.len() > 200 {
        let drop_n = list.len() - 200;
        list.drain(0..drop_n);
    }
}

fn resolve_skills_dir() -> PathBuf {
    if let Ok(path) = std::env::var("CLAWORK_SKILLS_DIR") {
        return PathBuf::from(path);
    }
    PathBuf::from("skills")
}

fn resolve_allowed_roots() -> Vec<PathBuf> {
    if let Ok(raw) = std::env::var("CLAWORK_ALLOWED_PATHS") {
        let roots: Vec<PathBuf> = raw
            .split([';', ','])
            .filter(|s| !s.trim().is_empty())
            .map(|s| PathBuf::from(s.trim()))
            .collect();
        if !roots.is_empty() {
            return roots;
        }
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    vec![cwd.clone(), cwd.join("data"), cwd.join("skills")]
}

fn resolve_mcp_client() -> Option<StdioMcpClient> {
    let command = std::env::var("CLAWORK_MCP_COMMAND").ok()?;
    let args = std::env::var("CLAWORK_MCP_ARGS")
        .map(|s| s.split_whitespace().map(|x| x.to_string()).collect())
        .unwrap_or_else(|_| Vec::new());
    Some(StdioMcpClient::new(command, args))
}

fn resolve_embedding_provider() -> Option<Arc<dyn EmbeddingProvider>> {
    let api_key = std::env::var("CLAWORK_OPENAI_API_KEY").ok()?;
    let model = std::env::var("CLAWORK_OPENAI_EMBEDDING_MODEL")
        .unwrap_or_else(|_| "text-embedding-3-small".into());
    Some(Arc::new(OpenAiEmbeddingProvider::new(api_key, model)))
}

fn resolve_local_api_addr() -> String {
    std::env::var("CLAWORK_LOCAL_API_ADDR").unwrap_or_else(|_| "127.0.0.1:4747".into())
}

fn resolve_seen_inbound_path() -> PathBuf {
    std::env::var("CLAWORK_INBOUND_DEDUPE_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/inbound_seen_ids.log"))
}

fn load_seen_inbound_ids(path: &Path, max_entries: usize) -> VecDeque<String> {
    let mut seen = VecDeque::new();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return seen;
    };

    for line in raw.lines() {
        let key = line.trim();
        if key.is_empty() {
            continue;
        }
        seen.push_back(key.to_string());
        while seen.len() > max_entries {
            seen.pop_front();
        }
    }
    seen
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .without_time()
        .init();

    let _ = std::fs::create_dir_all("data");
    let _ = std::fs::create_dir_all("data/browser");

    let daemon = DaemonService::default();
    let daemon_clone = daemon.clone();
    tauri::async_runtime::spawn(async move {
        let _ = daemon_clone
            .start(std::time::Duration::from_secs(60 * 30))
            .await;
    });

    let suggestions = Arc::new(RwLock::new(Vec::<ProactiveSuggestion>::new()));
    let suggestions_for_task = Arc::clone(&suggestions);
    let mut daemon_events = daemon.subscribe();
    tauri::async_runtime::spawn(async move {
        while let Ok(event) = daemon_events.recv().await {
            match event {
                DaemonEvent::Suggestion { at, text } => {
                    push_suggestion(&suggestions_for_task, ProactiveSuggestion { at, text });
                }
                DaemonEvent::DailyBriefingTick { at } => {
                    push_suggestion(
                        &suggestions_for_task,
                        ProactiveSuggestion {
                            at,
                            text: "Daily briefing tick received. Open briefing panel.".into(),
                        },
                    );
                }
                DaemonEvent::Heartbeat { .. } => {}
            }
        }
    });

    let registry = SkillRegistry::new(resolve_skills_dir());
    let _ = registry.reload();
    let watch_registry = registry.clone();
    tauri::async_runtime::spawn(async move {
        let _ = watch_registry.watch().await;
    });

    let runtime = WasmSkillRuntime::new(resolve_skills_dir(), registry.clone());

    let telegram_token = std::env::var("CLAWORK_TELEGRAM_BOT_TOKEN").ok();
    let whatsapp_token = std::env::var("CLAWORK_WHATSAPP_ACCESS_TOKEN").ok();
    let whatsapp_phone = std::env::var("CLAWORK_WHATSAPP_PHONE_ID").ok();

    let line_token = std::env::var("CLAWORK_LINE_CHANNEL_ACCESS_TOKEN").ok();
    let discord_webhook = std::env::var("CLAWORK_DISCORD_WEBHOOK_URL").ok();
    let slack_webhook = std::env::var("CLAWORK_SLACK_WEBHOOK_URL").ok();
    let slack_bot_token = std::env::var("CLAWORK_SLACK_BOT_TOKEN").ok();
    let slack_poll_channels: Vec<String> = std::env::var("CLAWORK_SLACK_POLL_CHANNELS")
        .map(|raw| {
            raw.split([',', ';', ' '])
                .filter_map(|item| {
                    let trimmed = item.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let signal_from = std::env::var("CLAWORK_SIGNAL_FROM").ok();
    let signal_command =
        std::env::var("CLAWORK_SIGNAL_COMMAND").unwrap_or_else(|_| "signal-cli".into());
    let requested_imessage = std::env::var("CLAWORK_ENABLE_IMESSAGE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let enable_imessage = requested_imessage && cfg!(target_os = "macos");

    let adapters = AdapterSet {
        telegram: telegram_token.map(TelegramAdapter::new),
        whatsapp: match (whatsapp_token, whatsapp_phone) {
            (Some(token), Some(phone_id)) => Some(WhatsAppCloudAdapter::new(token, phone_id)),
            _ => None,
        },
        line: line_token.map(LineAdapter::new),
        discord: discord_webhook.map(DiscordAdapter::new),
        slack: if slack_webhook.is_some()
            || (slack_bot_token.is_some() && !slack_poll_channels.is_empty())
        {
            Some(SlackAdapter::new(
                slack_webhook,
                slack_bot_token,
                slack_poll_channels,
            ))
        } else {
            None
        },
        signal: signal_from.map(|from| SignalAdapter::new(signal_command, from)),
        imessage: if enable_imessage {
            Some(IMessageAdapter)
        } else {
            None
        },
    };

    let mut init_errors = BTreeMap::<String, String>::new();
    let audit_store = match tauri::async_runtime::block_on(async {
        AuditStore::connect("sqlite://data/clawork_audit.db").await
    }) {
        Ok(store) => Some(store),
        Err(err) => {
            init_errors.insert("audit".into(), err.to_string());
            None
        }
    };

    let memory_store_db = match tauri::async_runtime::block_on(async {
        MemoryStore::connect("sqlite://data/clawork_memory.db").await
    }) {
        Ok(store) => Some(store),
        Err(err) => {
            init_errors.insert("memory".into(), err.to_string());
            None
        }
    };
    let operator_store = match tauri::async_runtime::block_on(async {
        OperatorStore::connect("sqlite://data/clawork_operator.db").await
    }) {
        Ok(store) => Some(store),
        Err(err) => {
            init_errors.insert("operator".into(), err.to_string());
            None
        }
    };

    let local_api_addr = resolve_local_api_addr();
    let local_api_token =
        ensure_cli_token().expect("failed to initialize local API token at data/cli.token");
    let mut local_api_tokens = HashMap::<String, Option<chrono::DateTime<chrono::Utc>>>::new();
    local_api_tokens.insert(local_api_token, None);
    let embedding_provider = resolve_embedding_provider();
    let seen_inbound_path = resolve_seen_inbound_path();
    let seen_inbound_ids = load_seen_inbound_ids(&seen_inbound_path, 2048);

    let app_state = AppState {
        daemon,
        permissions: PermissionEngine::default(),
        registry,
        runtime,
        adapters,
        audit: Arc::new(RwLock::new(audit_store)),
        fs: SandboxFsService::new(resolve_allowed_roots()),
        browser: PlaywrightSandbox::default(),
        office: OfficeService::default(),
        mcp: resolve_mcp_client(),
        memory: Arc::new(RwLock::new(memory_store_db)),
        suggestions,
        inbound_messages: Arc::new(RwLock::new(Vec::new())),
        seen_inbound_ids: Arc::new(RwLock::new(seen_inbound_ids)),
        seen_inbound_path,
        daemon_should_run: Arc::new(RwLock::new(true)),
        embedding_provider,
        local_api_tokens: Arc::new(RwLock::new(local_api_tokens)),
        local_api_addr,
        operator: Arc::new(RwLock::new(operator_store)),
        init_errors,
        http: Client::new(),
    };

    let local_api_state = Arc::new(app_state.clone());
    tauri::async_runtime::spawn(async move {
        if let Err(err) = run_local_api(local_api_state).await {
            tracing::error!("local api stopped: {}", err);
        }
    });

    let expiry_state = Arc::new(app_state.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if let Some(expired_at) = expiry_state.permissions.expire_elevation_if_needed() {
                let trace_id = Uuid::new_v4();
                append_audit(
                    &expiry_state,
                    &AuditEvent {
                        timestamp: Utc::now(),
                        actor: "daemon".into(),
                        action: ActionKind::ElevatedRequest,
                        target: Some("local-session".into()),
                        decision: "expired".into(),
                        reason: Some(format!(
                            "mode_transition=elevated->sandbox, expires_at={expired_at}"
                        )),
                        trace_id,
                    },
                )
                .await;
            }
        }
    });

    let health_state = Arc::new(app_state.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            if !*health_state.daemon_should_run.read() {
                continue;
            }
            if !health_state.daemon.snapshot().running {
                if let Err(err) = health_state
                    .daemon
                    .start(std::time::Duration::from_secs(60 * 30))
                    .await
                {
                    tracing::error!("daemon health restart failed: {}", err);
                } else {
                    push_suggestion(
                        &health_state.suggestions,
                        ProactiveSuggestion {
                            at: Utc::now(),
                            text: "Daemon auto-restarted after health check.".into(),
                        },
                    );
                }
            }
        }
    });

    let operator_state = Arc::new(app_state.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            let store = {
                let guard = operator_state.operator.read();
                guard.clone()
            };
            let Some(store) = store else {
                continue;
            };

            let due = match store.poll_due_tasks(Utc::now()).await {
                Ok(tasks) => tasks,
                Err(err) => {
                    tracing::warn!("operator due task poll failed: {}", err);
                    continue;
                }
            };

            for task in due {
                let _ = store.run_task_now(&task.id).await;
                push_suggestion(
                    &operator_state.suggestions,
                    ProactiveSuggestion {
                        at: Utc::now(),
                        text: format!("Operator scheduled task fired: {}", task.name),
                    },
                );
                append_audit(
                    &operator_state,
                    &AuditEvent {
                        timestamp: Utc::now(),
                        actor: "operator_scheduler".into(),
                        action: ActionKind::NetworkCall,
                        target: Some(format!("operator_task:{}", task.id)),
                        decision: "allowed".into(),
                        reason: Some(format!("cron={}, prompt={}", task.cron, task.prompt)),
                        trace_id: Uuid::new_v4(),
                    },
                )
                .await;
            }
        }
    });

    let inbound_state = Arc::new(app_state.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            if let Ok(messages) = poll_adapters_once(&inbound_state).await {
                let _ = ingest_inbound_messages(&inbound_state, "adapter-poller", messages).await;
            }
        }
    });

    let daemon_only = std::env::var("CLAWORK_DAEMON_ONLY")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if daemon_only {
        tracing::info!("running in daemon-only mode (no desktop window)");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            get_status,
            list_tasks,
            request_elevation,
            approve_action,
            run_skill,
            create_skill,
            list_skills,
            send_message,
            get_audit_events,
            fs_operate,
            browser_navigate,
            call_mcp_tool,
            office_create_excel,
            office_upload_graph_file,
            memory_store,
            memory_recent,
            memory_search,
            list_proactive_suggestions,
            get_daily_briefing,
            get_inbound_messages,
            operator_create_session,
            operator_list_sessions,
            operator_plan_session,
            operator_run_session,
            operator_timeline,
            operator_pending_approvals,
            operator_approve_proposal,
            operator_reject_proposal,
            operator_create_task,
            operator_list_tasks
        ])
        .run(tauri::generate_context!())
        .expect("failed to run tauri application");
}
