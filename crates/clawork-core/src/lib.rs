use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    FileRead,
    FileWrite,
    ShellExec,
    BrowserNav,
    NetworkCall,
    MessageSend,
    SkillInstall,
    SkillExecute,
    ElevatedRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRequest {
    pub kind: ActionKind,
    pub target: Option<String>,
    #[serde(default)]
    pub params: Value,
    pub trace_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestContext {
    pub actor: String,
    pub mode: PermissionMode,
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Sandbox,
    Elevated { expires_at: DateTime<Utc> },
}

impl PermissionMode {
    pub fn is_elevated_active(&self, now: DateTime<Utc>) -> bool {
        matches!(
            self,
            PermissionMode::Elevated { expires_at } if *expires_at > now
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny { reason: String },
    RequiresConfirmation { token: String, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: DateTime<Utc>,
    pub actor: String,
    pub action: ActionKind,
    pub target: Option<String>,
    pub decision: String,
    pub reason: Option<String>,
    pub trace_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRequest {
    pub abi_version: String,
    pub skill_id: String,
    #[serde(default)]
    pub input: Value,
    pub trace_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillResponse {
    pub abi_version: String,
    pub skill_id: String,
    pub success: bool,
    #[serde(default)]
    pub output: Value,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub adapter: String,
    pub from: String,
    pub content: String,
    pub received_at: DateTime<Utc>,
    pub external_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub adapter: String,
    pub to: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendResult {
    pub ok: bool,
    pub provider_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolArgs {
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub ok: bool,
    pub payload: Value,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub label: String,
    pub next_run: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStatus {
    pub app_mode: PermissionMode,
    pub daemon_running: bool,
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub loaded_skills: usize,
    pub adapters: BTreeMap<String, bool>,
    #[serde(default)]
    pub operator: Option<OperatorStatus>,
    #[serde(default)]
    pub init_errors: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorStatus {
    pub active_sessions: usize,
    pub pending_approvals: usize,
    pub scheduled_tasks: usize,
    pub connectors_health: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsOperationKind {
    ReadFile,
    WriteFile,
    DeleteFile,
    ListDirectory,
    CreateDirectory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsOperationRequest {
    pub kind: FsOperationKind,
    pub path: String,
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsEntry {
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsOperationResult {
    pub ok: bool,
    pub message: String,
    pub content: Option<String>,
    pub entries: Vec<FsEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserRunRequest {
    pub url: String,
    pub allow_domains: Vec<String>,
    pub headed: bool,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserRunResult {
    pub ok: bool,
    pub command: String,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfficeExcelRequest {
    pub output_path: String,
    pub sheet_name: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfficeGraphUploadRequest {
    pub graph_token: String,
    pub remote_path: String,
    pub mime_type: String,
    pub content_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfficeUploadResult {
    pub ok: bool,
    pub status: u16,
    pub response_body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProactiveSuggestion {
    pub at: DateTime<Utc>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyBriefing {
    pub generated_at: DateTime<Utc>,
    pub overview: String,
    pub tasks_due: usize,
    pub recent_audit_events: usize,
    pub memory_entries: usize,
    pub rationales: Vec<String>,
    #[serde(default)]
    pub rationale_sources: Vec<BriefingRationale>,
    pub suggestions: Vec<ProactiveSuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalProfile {
    Low,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainPolicy {
    pub domain: String,
    pub profile: ApprovalProfile,
    #[serde(default)]
    pub blocked_actions: Vec<ActionKind>,
    #[serde(default)]
    pub allow_actions: Vec<ActionKind>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperatorSessionState {
    Draft,
    Planned,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperatorStepState {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorSession {
    pub id: String,
    pub title: String,
    pub goal: String,
    pub state: OperatorSessionState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorStep {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub detail: String,
    pub state: OperatorStepState,
    pub trace_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperatorActionState {
    PendingApproval,
    Approved,
    Rejected,
    Executed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorActionProposal {
    pub id: String,
    pub session_id: String,
    pub step_id: Option<String>,
    pub action_kind: ActionKind,
    pub target: Option<String>,
    #[serde(default)]
    pub params: Value,
    pub state: OperatorActionState,
    pub approval_token: Option<String>,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorActionResult {
    pub id: String,
    pub action_id: String,
    pub ok: bool,
    #[serde(default)]
    pub output: Value,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorTimelineItem {
    pub at: DateTime<Utc>,
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorTaskTemplate {
    pub id: String,
    pub name: String,
    pub cron: String,
    pub prompt: String,
    pub target_project: Option<String>,
    pub enabled: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationRef {
    pub source_url: String,
    pub snippet_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectArtifact {
    pub id: String,
    pub project_id: String,
    pub path: String,
    pub mime: String,
    pub producer_step: Option<String>,
    #[serde(default)]
    pub citations: Vec<CitationRef>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorStatus {
    pub provider: String,
    pub connected: bool,
    pub account_id: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingRationale {
    pub source: String,
    pub value: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Denied,
    ConfirmationRequired,
    NotConfigured,
    ValidationError,
    Timeout,
    InternalError,
}

#[async_trait]
pub trait CapabilityGuard: Send + Sync {
    async fn authorize(&self, action: &ActionRequest, ctx: &RequestContext) -> Decision;
}

#[async_trait]
pub trait SkillRuntime: Send + Sync {
    async fn execute(&self, skill_id: &str, request: SkillRequest)
        -> anyhow::Result<SkillResponse>;
}

#[async_trait]
pub trait MessageAdapter: Send + Sync {
    async fn poll(&self) -> anyhow::Result<Vec<InboundMessage>>;
    async fn send(&self, message: OutboundMessage) -> anyhow::Result<SendResult>;
}

#[async_trait]
pub trait ToolCaller: Send + Sync {
    async fn call(&self, tool_name: &str, args: ToolArgs) -> anyhow::Result<ToolResult>;
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>>;
}

#[async_trait]
pub trait FilesystemService: Send + Sync {
    async fn operate(
        &self,
        op: FsOperationRequest,
        mode: PermissionMode,
    ) -> anyhow::Result<FsOperationResult>;
}

#[async_trait]
pub trait BrowserAutomationService: Send + Sync {
    async fn navigate(&self, req: BrowserRunRequest) -> anyhow::Result<BrowserRunResult>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn error_code_serializes_to_fixed_strings() {
        let code = ErrorCode::ConfirmationRequired;
        let json = serde_json::to_string(&code).expect("serialize error code");
        assert_eq!(json, "\"confirmation_required\"");
    }

    #[test]
    fn permission_mode_elevated_expiry_check() {
        let now = Utc::now();
        let mode = PermissionMode::Elevated {
            expires_at: now + Duration::seconds(60),
        };
        assert!(mode.is_elevated_active(now));
    }
}
