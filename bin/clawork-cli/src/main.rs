use anyhow::{anyhow, Context};
use clap::{Parser, Subcommand};
use clawork_core::{
    ActionKind, ApprovalProfile, BrowserRunRequest, FsOperationKind, FsOperationRequest,
    OfficeExcelRequest,
};
use reqwest::header::HeaderValue;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(name = "clawork", version, about = "Clawork CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Status,
    Ask {
        instruction: String,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        continue_on_error: bool,
    },
    Approve {
        token: String,
    },
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
    Fs {
        #[command(subcommand)]
        command: FsCommand,
    },
    Browser {
        url: String,
    },
    Mcp {
        tool: String,
        payload: Option<String>,
        #[arg(long)]
        route: Option<String>,
    },
    Memory {
        #[command(subcommand)]
        command: MemoryCommand,
    },
    Briefing {
        #[command(subcommand)]
        command: BriefingCommand,
    },
    Office {
        #[command(subcommand)]
        command: OfficeCommand,
    },
    Logs {
        #[command(subcommand)]
        command: LogsCommand,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Messages {
        #[command(subcommand)]
        command: MessagesCommand,
    },
    Mail {
        #[command(subcommand)]
        command: MailCommand,
    },
    Media {
        #[command(subcommand)]
        command: MediaCommand,
    },
    Research {
        #[command(subcommand)]
        command: ResearchCommand,
    },
    Operator {
        #[command(subcommand)]
        command: OperatorCommand,
    },
    Connectors {
        #[command(subcommand)]
        command: ConnectorsCommand,
    },
    Policy {
        #[command(subcommand)]
        command: PolicyCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DaemonCommand {
    Start,
    Stop,
    Restart,
    Serve,
}

#[derive(Debug, Subcommand)]
enum TokenCommand {
    Issue {
        #[arg(long)]
        ttl_seconds: Option<i64>,
    },
    Revoke {
        token: String,
    },
    Rotate {
        #[arg(long)]
        ttl_seconds: Option<i64>,
    },
}

#[derive(Debug, Subcommand)]
enum TaskCommand {
    Run { task_id: String },
}

#[derive(Debug, Subcommand)]
enum SkillCommand {
    List,
    Run {
        skill_id: String,
        payload: Option<String>,
    },
    Install {
        path: String,
    },
    Create {
        skill_id: String,
        name: String,
        description: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum LogsCommand {
    Tail,
}

#[derive(Debug, Subcommand)]
enum FsCommand {
    Read { path: String },
    Write { path: String, content: String },
    List { path: String },
    Delete { path: String },
    Mkdir { path: String },
}

#[derive(Debug, Subcommand)]
enum OfficeCommand {
    Excel {
        output_path: String,
        sheet_name: String,
    },
    Upload {
        remote_path: String,
        mime_type: String,
        base64_file: String,
    },
}

#[derive(Debug, Subcommand)]
enum MemoryCommand {
    Store { text: String },
    Search { query: String, limit: Option<i64> },
    Recent { limit: Option<i64> },
}

#[derive(Debug, Subcommand)]
enum BriefingCommand {
    Show,
    Suggestions { limit: Option<usize> },
}

#[derive(Debug, Subcommand)]
enum MessagesCommand {
    Send {
        adapter: String,
        to: String,
        content: String,
    },
    Inbound {
        limit: Option<usize>,
        adapter: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum MailCommand {
    Unreplied { limit: Option<usize> },
}

#[derive(Debug, Subcommand)]
enum MediaCommand {
    Image {
        prompt: String,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        size: Option<String>,
        #[arg(long)]
        output_path: Option<String>,
        #[arg(long)]
        project_id: Option<String>,
    },
    Video {
        prompt: String,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        seconds: Option<u32>,
        #[arg(long)]
        output_path: Option<String>,
        #[arg(long)]
        project_id: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum ResearchCommand {
    Create {
        question: String,
        source_urls_json: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        project_id: Option<String>,
    },
    Show {
        id: String,
    },
    Report {
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Show,
    Set { key: String, value: String },
}

#[derive(Debug, Subcommand)]
enum OperatorCommand {
    Session {
        #[command(subcommand)]
        command: OperatorSessionCommand,
    },
    Approvals {
        #[command(subcommand)]
        command: OperatorApprovalsCommand,
    },
    Task {
        #[command(subcommand)]
        command: OperatorTaskCommand,
    },
    Project {
        #[command(subcommand)]
        command: OperatorProjectCommand,
    },
}

#[derive(Debug, Subcommand)]
enum OperatorSessionCommand {
    Create { title: String, goal: String },
    List { limit: Option<usize> },
    Show { id: String },
    Plan { id: String, steps_json: String },
    Run { id: String },
    Timeline { id: String, limit: Option<usize> },
}

#[derive(Debug, Subcommand)]
enum OperatorApprovalsCommand {
    List { limit: Option<usize> },
    Approve { id: String, actor: Option<String> },
    Reject { id: String, actor: Option<String> },
}

#[derive(Debug, Subcommand)]
enum OperatorTaskCommand {
    Create {
        name: String,
        cron: String,
        prompt: String,
        target_project: Option<String>,
        #[arg(long)]
        disabled: bool,
    },
    List {
        limit: Option<usize>,
    },
    RunNow {
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum OperatorProjectCommand {
    Create {
        name: String,
        #[arg(long)]
        description: Option<String>,
    },
    List {
        limit: Option<usize>,
    },
    Artifacts {
        id: String,
        limit: Option<usize>,
    },
}

#[derive(Debug, Subcommand)]
enum ConnectorsCommand {
    Status,
    Auth {
        provider: String,
        redirect_uri: Option<String>,
        scopes_csv: Option<String>,
    },
    GoogleSheetsAppend {
        spreadsheet_id: String,
        values_json: String,
        #[arg(long)]
        sheet_name: Option<String>,
        #[arg(long)]
        value_input_option: Option<String>,
    },
    GoogleDriveCreate {
        name: String,
        content: String,
        #[arg(long)]
        parent_id: Option<String>,
        #[arg(long)]
        mime_type: Option<String>,
    },
    NotionSearch {
        query: String,
        #[arg(long)]
        page_size: Option<usize>,
    },
    NotionPageCreate {
        title: String,
        #[arg(long)]
        content: Option<String>,
        #[arg(long)]
        parent_page_id: Option<String>,
        #[arg(long)]
        parent_database_id: Option<String>,
    },
    SlackPost {
        channel: String,
        text: String,
    },
    SlackHistory {
        channel: String,
        #[arg(long)]
        limit: Option<usize>,
    },
}

#[derive(Debug, Subcommand)]
enum PolicyCommand {
    Domain {
        #[command(subcommand)]
        command: PolicyDomainCommand,
    },
}

#[derive(Debug, Subcommand)]
enum PolicyDomainCommand {
    Set {
        domain: String,
        profile: String,
        blocked_csv: Option<String>,
        allow_csv: Option<String>,
    },
    List,
}

#[derive(Debug, Deserialize)]
struct CommandErrorBody {
    code: String,
    message: String,
    confirmation_token: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct FsOperateApiReq {
    #[serde(flatten)]
    op: FsOperationRequest,
    approval_token: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct BrowserNavigateApiReq {
    #[serde(flatten)]
    req: BrowserRunRequest,
    approval_token: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct OfficeExcelApiReq {
    #[serde(flatten)]
    req: OfficeExcelRequest,
    approval_token: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct OfficeUploadApiReq {
    graph_token: String,
    remote_path: String,
    mime_type: String,
    content_base64: String,
    approval_token: Option<String>,
}

#[derive(Clone)]
struct LocalApiClient {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl LocalApiClient {
    fn new() -> anyhow::Result<Self> {
        let base = std::env::var("CLAWORK_LOCAL_API_BASE")
            .unwrap_or_else(|_| "http://127.0.0.1:4747".to_string());
        let token = load_cli_token()?;
        Ok(Self {
            http: reqwest::Client::new(),
            base,
            token,
        })
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    fn auth_header_value(&self) -> anyhow::Result<HeaderValue> {
        HeaderValue::from_str(&self.token).context("invalid cli token header value")
    }

    async fn get_json<T>(&self, path: &str, query: Option<Vec<(&str, String)>>) -> anyhow::Result<T>
    where
        T: DeserializeOwned,
    {
        let mut req = self
            .http
            .get(self.url(path))
            .header("X-Clawork-Token", self.auth_header_value()?);

        if let Some(query) = query {
            req = req.query(&query);
        }

        let res = req.send().await.context("local api get request failed")?;
        decode_response(res).await
    }

    async fn post_json<B, T>(&self, path: &str, body: &B) -> anyhow::Result<T>
    where
        B: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        let res = self
            .http
            .post(self.url(path))
            .header("X-Clawork-Token", self.auth_header_value()?)
            .json(body)
            .send()
            .await
            .context("local api post request failed")?;
        decode_response(res).await
    }

    async fn approve(&self, token: &str) -> anyhow::Result<bool> {
        self.post_json(
            "/v1/actions/approve",
            &serde_json::json!({ "token": token }),
        )
        .await
    }
}

async fn decode_response<T>(res: reqwest::Response) -> anyhow::Result<T>
where
    T: DeserializeOwned,
{
    let status = res.status();
    if status.is_success() {
        return res
            .json::<T>()
            .await
            .with_context(|| format!("failed to parse success response ({status})"));
    }

    let body_text = res
        .text()
        .await
        .unwrap_or_else(|_| "<no response body>".to_string());
    if let Ok(err) = serde_json::from_str::<CommandErrorBody>(&body_text) {
        let token_hint = err
            .confirmation_token
            .as_deref()
            .map(|t| format!(", confirmation_token={t}"))
            .unwrap_or_default();
        return Err(anyhow!(
            "api error [{}] {}: {}{}",
            status.as_u16(),
            err.code,
            err.message,
            token_hint
        ));
    }

    Err(anyhow!("api error [{}]: {}", status.as_u16(), body_text))
}

fn extract_confirmation_token(err: &anyhow::Error) -> Option<String> {
    let msg = err.to_string();
    if !msg.contains("confirmation_required") {
        return None;
    }
    let marker = "confirmation_token=";
    let idx = msg.find(marker)?;
    let token = msg[idx + marker.len()..]
        .split([',', ' ', '\n', '\r'])
        .find(|s| !s.is_empty())?
        .to_string();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

async fn with_approval_retry<T, F, Fut>(client: &LocalApiClient, mut call: F) -> anyhow::Result<T>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    let mut token: Option<String> = None;
    for _ in 0..2 {
        match call(token.clone()).await {
            Ok(value) => return Ok(value),
            Err(err) => {
                if let Some(next_token) = extract_confirmation_token(&err) {
                    let approved = client.approve(&next_token).await?;
                    if approved {
                        token = Some(next_token);
                        continue;
                    }
                }
                return Err(err);
            }
        }
    }
    Err(anyhow!("action still requires confirmation after retry"))
}

fn load_cli_token() -> anyhow::Result<String> {
    if let Ok(token) = std::env::var("CLAWORK_CLI_TOKEN") {
        if !token.trim().is_empty() {
            return Ok(token.trim().to_string());
        }
    }

    let path = std::env::var("CLAWORK_CLI_TOKEN_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/cli.token"));
    let token = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "failed to read token file '{}'; start desktop app first or set CLAWORK_CLI_TOKEN",
            path.display()
        )
    })?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!(
            "token file '{}' is empty; restart desktop app",
            path.display()
        ));
    }
    Ok(token)
}

fn parse_json_payload(raw: Option<String>, default_obj: bool) -> anyhow::Result<Value> {
    match raw {
        Some(s) if !s.trim().is_empty() => {
            serde_json::from_str::<Value>(&s).context("payload must be valid JSON")
        }
        _ if default_obj => Ok(serde_json::json!({})),
        _ => Ok(Value::Null),
    }
}

fn parse_steps_json(raw: &str) -> anyhow::Result<Vec<String>> {
    serde_json::from_str::<Vec<String>>(raw)
        .context("steps_json must be a JSON array of strings, e.g. [\"step1\",\"step2\"]")
}

fn parse_approval_profile(raw: &str) -> anyhow::Result<ApprovalProfile> {
    serde_json::from_str::<ApprovalProfile>(&format!("\"{}\"", raw.trim().to_lowercase()))
        .context("profile must be 'low' or 'high'")
}

fn parse_action_list_csv(raw: Option<String>) -> anyhow::Result<Vec<ActionKind>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in raw.split(',') {
        let name = item.trim();
        if name.is_empty() {
            continue;
        }
        let action = serde_json::from_str::<ActionKind>(&format!("\"{}\"", name.to_lowercase()))
            .with_context(|| format!("unknown action kind '{name}'"))?;
        out.push(action);
    }
    Ok(out)
}

fn parse_scopes_csv(raw: Option<String>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_skill_manifest(path: &str) -> anyhow::Result<(String, String, Option<String>)> {
    let input = Path::new(path);
    let manifest_path = if input.is_dir() {
        input.join("manifest.json")
    } else {
        input.to_path_buf()
    };

    let raw = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let v: Value = serde_json::from_str(&raw)
        .with_context(|| format!("invalid JSON in {}", manifest_path.display()))?;

    let skill_id = v
        .get("skill_id")
        .or_else(|| v.get("id"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            manifest_path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(ToString::to_string)
        })
        .ok_or_else(|| anyhow!("skill_id/id missing in {}", manifest_path.display()))?;

    let name = v
        .get("name")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| skill_id.clone());
    let description = v
        .get("description")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    Ok((skill_id, name, description))
}

fn print_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn start_daemon_only_process() -> anyhow::Result<()> {
    let desktop_bin =
        std::env::var("CLAWORK_DESKTOP_BIN").unwrap_or_else(|_| "clawork-desktop".to_string());
    let mut child = std::process::Command::new(desktop_bin)
        .env("CLAWORK_DAEMON_ONLY", "1")
        .spawn()
        .context("failed to start daemon-only desktop process")?;
    let status = child.wait().context("failed to wait daemon-only process")?;
    if !status.success() {
        return Err(anyhow!("daemon-only process exited with {status}"));
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Commands::Daemon {
        command: DaemonCommand::Serve,
    } = &cli.command
    {
        start_daemon_only_process()?;
        return Ok(());
    }
    let client = LocalApiClient::new()?;

    match cli.command {
        Commands::Status => {
            let res: Value = client.get_json("/v1/status", None).await?;
            print_json(&res)?;
        }
        Commands::Ask {
            instruction,
            dry_run,
            continue_on_error,
        } => {
            let res: Value = with_approval_retry(&client, |approval_token| {
                let body = serde_json::json!({
                    "instruction": instruction.clone(),
                    "dry_run": dry_run,
                    "continue_on_error": continue_on_error,
                    "approval_token": approval_token
                });
                let client = client.clone();
                async move { client.post_json("/v1/nl/execute", &body).await }
            })
            .await?;
            print_json(&res)?;
        }
        Commands::Approve { token } => {
            let approved = client.approve(&token).await?;
            print_json(&serde_json::json!({ "approved": approved, "token": token }))?;
        }
        Commands::Token { command } => match command {
            TokenCommand::Issue { ttl_seconds } => {
                let res: Value = client
                    .post_json(
                        "/v1/auth/token/issue",
                        &serde_json::json!({ "ttl_seconds": ttl_seconds }),
                    )
                    .await?;
                print_json(&res)?;
            }
            TokenCommand::Revoke { token } => {
                let revoked: bool = client
                    .post_json(
                        "/v1/auth/token/revoke",
                        &serde_json::json!({ "token": token }),
                    )
                    .await?;
                print_json(&serde_json::json!({ "revoked": revoked }))?;
            }
            TokenCommand::Rotate { ttl_seconds } => {
                let res: Value = client
                    .post_json(
                        "/v1/auth/token/rotate",
                        &serde_json::json!({ "ttl_seconds": ttl_seconds }),
                    )
                    .await?;
                print_json(&res)?;
            }
        },
        Commands::Daemon { command } => match command {
            DaemonCommand::Start => {
                let res: Value = client
                    .post_json("/v1/daemon/start", &serde_json::json!({}))
                    .await?;
                print_json(&res)?;
            }
            DaemonCommand::Stop => {
                let res: Value = client
                    .post_json("/v1/daemon/stop", &serde_json::json!({}))
                    .await?;
                print_json(&res)?;
            }
            DaemonCommand::Restart => {
                let res: Value = client
                    .post_json("/v1/daemon/restart", &serde_json::json!({}))
                    .await?;
                print_json(&res)?;
            }
            DaemonCommand::Serve => unreachable!(),
        },
        Commands::Task { command } => match command {
            TaskCommand::Run { task_id } => {
                let res: Value = client
                    .post_json("/v1/tasks/run", &serde_json::json!({ "task_id": task_id }))
                    .await?;
                print_json(&res)?;
            }
        },
        Commands::Skill { command } => match command {
            SkillCommand::List => {
                let res: Value = client.get_json("/v1/skills/list", None).await?;
                print_json(&res)?;
            }
            SkillCommand::Run { skill_id, payload } => {
                let input = parse_json_payload(payload, true)?;
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "skill_id": skill_id.clone(),
                        "input": input.clone(),
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move { client.post_json("/v1/skills/run", &body).await }
                })
                .await?;
                print_json(&res)?;
            }
            SkillCommand::Install { path } => {
                let (skill_id, name, description) = parse_skill_manifest(&path)?;
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "skill_id": skill_id.clone(),
                        "name": name.clone(),
                        "description": description.clone(),
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move { client.post_json("/v1/skills/create", &body).await }
                })
                .await?;
                print_json(&res)?;
            }
            SkillCommand::Create {
                skill_id,
                name,
                description,
            } => {
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "skill_id": skill_id.clone(),
                        "name": name.clone(),
                        "description": description.clone(),
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move { client.post_json("/v1/skills/create", &body).await }
                })
                .await?;
                print_json(&res)?;
            }
        },
        Commands::Fs { command } => {
            let op = match command {
                FsCommand::Read { path } => FsOperationRequest {
                    kind: FsOperationKind::ReadFile,
                    path,
                    content: None,
                },
                FsCommand::Write { path, content } => FsOperationRequest {
                    kind: FsOperationKind::WriteFile,
                    path,
                    content: Some(content),
                },
                FsCommand::List { path } => FsOperationRequest {
                    kind: FsOperationKind::ListDirectory,
                    path,
                    content: None,
                },
                FsCommand::Delete { path } => FsOperationRequest {
                    kind: FsOperationKind::DeleteFile,
                    path,
                    content: None,
                },
                FsCommand::Mkdir { path } => FsOperationRequest {
                    kind: FsOperationKind::CreateDirectory,
                    path,
                    content: None,
                },
            };
            let res: Value = with_approval_retry(&client, |approval_token| {
                let req = FsOperateApiReq {
                    op: op.clone(),
                    approval_token,
                };
                let client = client.clone();
                async move { client.post_json("/v1/fs/operate", &req).await }
            })
            .await?;
            print_json(&res)?;
        }
        Commands::Browser { url } => {
            let req = BrowserRunRequest {
                url,
                allow_domains: vec![],
                headed: false,
                timeout_seconds: 30,
            };
            let res: Value = with_approval_retry(&client, |approval_token| {
                let body = BrowserNavigateApiReq {
                    req: req.clone(),
                    approval_token,
                };
                let client = client.clone();
                async move { client.post_json("/v1/browser/navigate", &body).await }
            })
            .await?;
            print_json(&res)?;
        }
        Commands::Mcp {
            tool,
            payload,
            route,
        } => {
            let parsed = parse_json_payload(payload, true)?;
            let res: Value = with_approval_retry(&client, |approval_token| {
                let body = serde_json::json!({
                    "tool_name": tool.clone(),
                    "payload": parsed.clone(),
                    "route": route.clone(),
                    "approval_token": approval_token
                });
                let client = client.clone();
                async move { client.post_json("/v1/mcp/call", &body).await }
            })
            .await?;
            print_json(&res)?;
        }
        Commands::Memory { command } => match command {
            MemoryCommand::Store { text } => {
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "text": text.clone(),
                        "embedding": Value::Null,
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move { client.post_json("/v1/memory/store", &body).await }
                })
                .await?;
                print_json(&res)?;
            }
            MemoryCommand::Search { query, limit } => {
                let res: Value = client
                    .post_json(
                        "/v1/memory/search",
                        &serde_json::json!({
                            "query": query,
                            "limit": limit
                        }),
                    )
                    .await?;
                print_json(&res)?;
            }
            MemoryCommand::Recent { limit } => {
                let query = limit.map(|v| vec![("limit", v.to_string())]);
                let res: Value = client.get_json("/v1/memory/recent", query).await?;
                print_json(&res)?;
            }
        },
        Commands::Briefing { command } => match command {
            BriefingCommand::Show => {
                let res: Value = client.get_json("/v1/briefing", None).await?;
                print_json(&res)?;
            }
            BriefingCommand::Suggestions { limit } => {
                let query = limit.map(|v| vec![("limit", v.to_string())]);
                let res: Value = client.get_json("/v1/suggestions", query).await?;
                print_json(&res)?;
            }
        },
        Commands::Office { command } => match command {
            OfficeCommand::Excel {
                output_path,
                sheet_name,
            } => {
                let req = OfficeExcelRequest {
                    output_path,
                    sheet_name,
                    headers: vec![],
                    rows: vec![],
                };
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = OfficeExcelApiReq {
                        req: req.clone(),
                        approval_token,
                    };
                    let client = client.clone();
                    async move { client.post_json("/v1/office/excel", &body).await }
                })
                .await?;
                print_json(&res)?;
            }
            OfficeCommand::Upload {
                remote_path,
                mime_type,
                base64_file,
            } => {
                let graph_token = std::env::var("CLAWORK_GRAPH_TOKEN")
                    .context("set CLAWORK_GRAPH_TOKEN for office upload")?;
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = OfficeUploadApiReq {
                        graph_token: graph_token.clone(),
                        remote_path: remote_path.clone(),
                        mime_type: mime_type.clone(),
                        content_base64: base64_file.clone(),
                        approval_token,
                    };
                    let client = client.clone();
                    async move { client.post_json("/v1/office/upload", &body).await }
                })
                .await?;
                print_json(&res)?;
            }
        },
        Commands::Messages { command } => match command {
            MessagesCommand::Send {
                adapter,
                to,
                content,
            } => {
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "adapter": adapter.clone(),
                        "to": to.clone(),
                        "content": content.clone(),
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move { client.post_json("/v1/messages/send", &body).await }
                })
                .await?;
                print_json(&res)?;
            }
            MessagesCommand::Inbound { limit, adapter } => {
                let mut query = Vec::new();
                if let Some(limit) = limit {
                    query.push(("limit", limit.to_string()));
                }
                if let Some(adapter) = adapter {
                    query.push(("adapter", adapter));
                }
                let res: Value = client.get_json("/v1/messages/inbound", Some(query)).await?;
                print_json(&res)?;
            }
        },
        Commands::Mail { command } => match command {
            MailCommand::Unreplied { limit } => {
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let mut query = Vec::new();
                    if let Some(limit) = limit {
                        query.push(("limit", limit.to_string()));
                    }
                    if let Some(token) = approval_token {
                        query.push(("approval_token", token));
                    }
                    let client = client.clone();
                    async move {
                        client
                            .get_json("/v1/mail/inbox/unreplied", Some(query))
                            .await
                    }
                })
                .await?;
                print_json(&res)?;
            }
        },
        Commands::Media { command } => match command {
            MediaCommand::Image {
                prompt,
                provider,
                model,
                size,
                output_path,
                project_id,
            } => {
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "prompt": prompt.clone(),
                        "provider": provider.clone(),
                        "model": model.clone(),
                        "size": size.clone(),
                        "seconds": Value::Null,
                        "output_path": output_path.clone(),
                        "project_id": project_id.clone(),
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move { client.post_json("/v1/media/image", &body).await }
                })
                .await?;
                print_json(&res)?;
            }
            MediaCommand::Video {
                prompt,
                provider,
                model,
                seconds,
                output_path,
                project_id,
            } => {
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "prompt": prompt.clone(),
                        "provider": provider.clone(),
                        "model": model.clone(),
                        "size": Value::Null,
                        "seconds": seconds,
                        "output_path": output_path.clone(),
                        "project_id": project_id.clone(),
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move { client.post_json("/v1/media/video", &body).await }
                })
                .await?;
                print_json(&res)?;
            }
        },
        Commands::Research { command } => match command {
            ResearchCommand::Create {
                question,
                source_urls_json,
                title,
                project_id,
            } => {
                let source_urls: Vec<String> = serde_json::from_str(&source_urls_json)
                    .with_context(|| "source_urls_json must be a JSON array of strings")?;
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "title": title.clone(),
                        "question": question.clone(),
                        "source_urls": source_urls.clone(),
                        "project_id": project_id.clone(),
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move { client.post_json("/v1/research/jobs", &body).await }
                })
                .await?;
                print_json(&res)?;
            }
            ResearchCommand::Show { id } => {
                let res: Value = client
                    .get_json(&format!("/v1/research/jobs/{id}"), None)
                    .await?;
                print_json(&res)?;
            }
            ResearchCommand::Report { id } => {
                let res: Value = client
                    .get_json(&format!("/v1/research/jobs/{id}/report"), None)
                    .await?;
                print_json(&res)?;
            }
        },
        Commands::Logs { command } => match command {
            LogsCommand::Tail => {
                let res: Value = client
                    .get_json("/v1/logs", Some(vec![("limit", "200".to_string())]))
                    .await?;
                print_json(&res)?;
            }
        },
        Commands::Config { command } => match command {
            ConfigCommand::Show => {
                let res: Value = client.get_json("/v1/config", None).await?;
                print_json(&res)?;
            }
            ConfigCommand::Set { key, value } => {
                let res: Value = client
                    .post_json(
                        "/v1/config/set",
                        &serde_json::json!({
                            "key": key,
                            "value": value
                        }),
                    )
                    .await?;
                print_json(&res)?;
            }
        },
        Commands::Operator { command } => match command {
            OperatorCommand::Session { command } => match command {
                OperatorSessionCommand::Create { title, goal } => {
                    let res: Value = client
                        .post_json(
                            "/v1/operator/sessions",
                            &serde_json::json!({
                                "title": title,
                                "goal": goal
                            }),
                        )
                        .await?;
                    print_json(&res)?;
                }
                OperatorSessionCommand::List { limit } => {
                    let query = limit.map(|v| vec![("limit", v.to_string())]);
                    let res: Value = client.get_json("/v1/operator/sessions", query).await?;
                    print_json(&res)?;
                }
                OperatorSessionCommand::Show { id } => {
                    let res: Value = client
                        .get_json(&format!("/v1/operator/sessions/{id}"), None)
                        .await?;
                    print_json(&res)?;
                }
                OperatorSessionCommand::Plan { id, steps_json } => {
                    let steps = parse_steps_json(&steps_json)?;
                    let res: Value = client
                        .post_json(
                            &format!("/v1/operator/sessions/{id}/plan"),
                            &serde_json::json!({ "steps": steps }),
                        )
                        .await?;
                    print_json(&res)?;
                }
                OperatorSessionCommand::Run { id } => {
                    let res: Value = client
                        .post_json(
                            &format!("/v1/operator/sessions/{id}/run"),
                            &serde_json::json!({}),
                        )
                        .await?;
                    print_json(&res)?;
                }
                OperatorSessionCommand::Timeline { id, limit } => {
                    let query = limit.map(|v| vec![("limit", v.to_string())]);
                    let res: Value = client
                        .get_json(&format!("/v1/operator/sessions/{id}/timeline"), query)
                        .await?;
                    print_json(&res)?;
                }
            },
            OperatorCommand::Approvals { command } => match command {
                OperatorApprovalsCommand::List { limit } => {
                    let query = limit.map(|v| vec![("limit", v.to_string())]);
                    let res: Value = client
                        .get_json("/v1/operator/approvals/pending", query)
                        .await?;
                    print_json(&res)?;
                }
                OperatorApprovalsCommand::Approve { id, actor } => {
                    let res: Value = client
                        .post_json(
                            &format!("/v1/operator/actions/{id}/approve"),
                            &serde_json::json!({ "actor": actor }),
                        )
                        .await?;
                    print_json(&res)?;
                }
                OperatorApprovalsCommand::Reject { id, actor } => {
                    let res: Value = client
                        .post_json(
                            &format!("/v1/operator/actions/{id}/reject"),
                            &serde_json::json!({ "actor": actor }),
                        )
                        .await?;
                    print_json(&res)?;
                }
            },
            OperatorCommand::Task { command } => match command {
                OperatorTaskCommand::Create {
                    name,
                    cron,
                    prompt,
                    target_project,
                    disabled,
                } => {
                    let res: Value = client
                        .post_json(
                            "/v1/operator/tasks",
                            &serde_json::json!({
                                "name": name,
                                "cron": cron,
                                "prompt": prompt,
                                "target_project": target_project,
                                "enabled": !disabled
                            }),
                        )
                        .await?;
                    print_json(&res)?;
                }
                OperatorTaskCommand::List { limit } => {
                    let query = limit.map(|v| vec![("limit", v.to_string())]);
                    let res: Value = client.get_json("/v1/operator/tasks", query).await?;
                    print_json(&res)?;
                }
                OperatorTaskCommand::RunNow { id } => {
                    let res: Value = client
                        .post_json(
                            &format!("/v1/operator/tasks/{id}/run-now"),
                            &serde_json::json!({}),
                        )
                        .await?;
                    print_json(&res)?;
                }
            },
            OperatorCommand::Project { command } => match command {
                OperatorProjectCommand::Create { name, description } => {
                    let res: Value = client
                        .post_json(
                            "/v1/operator/projects",
                            &serde_json::json!({
                                "name": name,
                                "description": description
                            }),
                        )
                        .await?;
                    print_json(&res)?;
                }
                OperatorProjectCommand::List { limit } => {
                    let query = limit.map(|v| vec![("limit", v.to_string())]);
                    let res: Value = client.get_json("/v1/operator/projects", query).await?;
                    print_json(&res)?;
                }
                OperatorProjectCommand::Artifacts { id, limit } => {
                    let query = limit.map(|v| vec![("limit", v.to_string())]);
                    let res: Value = client
                        .get_json(&format!("/v1/operator/projects/{id}/artifacts"), query)
                        .await?;
                    print_json(&res)?;
                }
            },
        },
        Commands::Connectors { command } => match command {
            ConnectorsCommand::Status => {
                let res: Value = client.get_json("/v1/connectors/status", None).await?;
                print_json(&res)?;
            }
            ConnectorsCommand::Auth {
                provider,
                redirect_uri,
                scopes_csv,
            } => {
                let scopes = parse_scopes_csv(scopes_csv);
                let res: Value = client
                    .post_json(
                        &format!("/v1/connectors/{provider}/oauth/start"),
                        &serde_json::json!({
                            "redirect_uri": redirect_uri,
                            "scopes": scopes
                        }),
                    )
                    .await?;
                print_json(&res)?;
            }
            ConnectorsCommand::GoogleSheetsAppend {
                spreadsheet_id,
                values_json,
                sheet_name,
                value_input_option,
            } => {
                let values: Vec<Vec<String>> = serde_json::from_str(&values_json)
                    .with_context(|| "values_json must be a JSON array of rows")?;
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "spreadsheet_id": spreadsheet_id.clone(),
                        "sheet_name": sheet_name.clone(),
                        "values": values.clone(),
                        "value_input_option": value_input_option.clone(),
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move {
                        client
                            .post_json("/v1/connectors/google/sheets/append", &body)
                            .await
                    }
                })
                .await?;
                print_json(&res)?;
            }
            ConnectorsCommand::GoogleDriveCreate {
                name,
                content,
                parent_id,
                mime_type,
            } => {
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "name": name.clone(),
                        "content": content.clone(),
                        "parent_id": parent_id.clone(),
                        "mime_type": mime_type.clone(),
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move {
                        client
                            .post_json("/v1/connectors/google/drive/create", &body)
                            .await
                    }
                })
                .await?;
                print_json(&res)?;
            }
            ConnectorsCommand::NotionSearch { query, page_size } => {
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "query": query.clone(),
                        "page_size": page_size,
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move {
                        client
                            .post_json("/v1/connectors/notion/search", &body)
                            .await
                    }
                })
                .await?;
                print_json(&res)?;
            }
            ConnectorsCommand::NotionPageCreate {
                title,
                content,
                parent_page_id,
                parent_database_id,
            } => {
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "title": title.clone(),
                        "content": content.clone(),
                        "parent_page_id": parent_page_id.clone(),
                        "parent_database_id": parent_database_id.clone(),
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move {
                        client
                            .post_json("/v1/connectors/notion/page/create", &body)
                            .await
                    }
                })
                .await?;
                print_json(&res)?;
            }
            ConnectorsCommand::SlackPost { channel, text } => {
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "channel": channel.clone(),
                        "text": text.clone(),
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move { client.post_json("/v1/connectors/slack/post", &body).await }
                })
                .await?;
                print_json(&res)?;
            }
            ConnectorsCommand::SlackHistory { channel, limit } => {
                let res: Value = with_approval_retry(&client, |approval_token| {
                    let body = serde_json::json!({
                        "channel": channel.clone(),
                        "limit": limit,
                        "approval_token": approval_token
                    });
                    let client = client.clone();
                    async move {
                        client
                            .post_json("/v1/connectors/slack/history", &body)
                            .await
                    }
                })
                .await?;
                print_json(&res)?;
            }
        },
        Commands::Policy { command } => match command {
            PolicyCommand::Domain { command } => match command {
                PolicyDomainCommand::Set {
                    domain,
                    profile,
                    blocked_csv,
                    allow_csv,
                } => {
                    let profile = parse_approval_profile(&profile)?;
                    let blocked_actions = parse_action_list_csv(blocked_csv)?;
                    let allow_actions = parse_action_list_csv(allow_csv)?;
                    let res: Value = client
                        .post_json(
                            "/v1/policies/domain",
                            &serde_json::json!({
                                "domain": domain,
                                "profile": profile,
                                "blocked_actions": blocked_actions,
                                "allow_actions": allow_actions
                            }),
                        )
                        .await?;
                    print_json(&res)?;
                }
                PolicyDomainCommand::List => {
                    let res: Value = client.get_json("/v1/policies/domain", None).await?;
                    print_json(&res)?;
                }
            },
        },
    }

    Ok(())
}
