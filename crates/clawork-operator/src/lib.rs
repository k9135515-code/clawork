use anyhow::Context;
use chrono::{DateTime, Utc};
use clawork_core::{
    ActionKind, ApprovalProfile, CitationRef, ConnectorStatus, DomainPolicy,
    OperatorActionProposal, OperatorActionResult, OperatorActionState, OperatorSession,
    OperatorSessionState, OperatorStep, OperatorTaskTemplate, OperatorTimelineItem,
    ProjectArtifact, ProjectInfo,
};
use cron::Schedule;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, SqlitePool};
use std::collections::BTreeMap;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Clone)]
pub struct OperatorStore {
    pool: SqlitePool,
}

#[derive(Debug, Clone)]
pub struct NewOperatorTask {
    pub name: String,
    pub cron: String,
    pub prompt: String,
    pub target_project: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct OAuthCallbackInput {
    pub account_id: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchJobRecord {
    pub id: String,
    pub project_id: Option<String>,
    pub title: String,
    pub question: String,
    pub state: String,
    pub source_urls: Vec<String>,
    pub report: Option<Value>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl OperatorStore {
    pub async fn connect(database_url: &str) -> anyhow::Result<Self> {
        let pool = SqlitePool::connect(database_url)
            .await
            .with_context(|| format!("connect operator db: {database_url}"))?;
        let store = Self { pool };
        store.init().await?;
        Ok(store)
    }

    async fn init(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS operator_sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                goal TEXT NOT NULL,
                state TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS operator_steps (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                title TEXT NOT NULL,
                detail TEXT NOT NULL,
                state TEXT NOT NULL,
                trace_id TEXT,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS operator_action_proposals (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                step_id TEXT,
                action_kind TEXT NOT NULL,
                target TEXT,
                params_json TEXT NOT NULL,
                state TEXT NOT NULL,
                approval_token TEXT,
                reason TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS operator_action_results (
                id TEXT PRIMARY KEY,
                action_id TEXT NOT NULL,
                ok INTEGER NOT NULL,
                output_json TEXT NOT NULL,
                error TEXT,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS operator_approvals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                action_id TEXT NOT NULL,
                state TEXT NOT NULL,
                token TEXT,
                actor TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS operator_tasks (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                cron TEXT NOT NULL,
                prompt TEXT NOT NULL,
                target_project TEXT,
                enabled INTEGER NOT NULL,
                next_run_at TEXT,
                last_run_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS domain_policies (
                domain TEXT PRIMARY KEY,
                profile TEXT NOT NULL,
                blocked_actions_json TEXT NOT NULL,
                allow_actions_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS connector_accounts (
                provider TEXT PRIMARY KEY,
                connected INTEGER NOT NULL,
                account_id TEXT,
                access_token TEXT,
                refresh_token TEXT,
                scopes_json TEXT,
                last_error TEXT,
                updated_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS project_artifacts (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                path TEXT NOT NULL,
                mime TEXT NOT NULL,
                producer_step TEXT,
                citations_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS research_jobs (
                id TEXT PRIMARY KEY,
                project_id TEXT,
                title TEXT NOT NULL,
                question TEXT NOT NULL,
                state TEXT NOT NULL,
                source_urls_json TEXT NOT NULL,
                report_json TEXT,
                error TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                completed_at TEXT
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        // Backward-compatible migration for existing databases created before project linkage.
        let _ = sqlx::query("ALTER TABLE research_jobs ADD COLUMN project_id TEXT")
            .execute(&self.pool)
            .await;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_operator_sessions_state_updated ON operator_sessions(state, updated_at DESC)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_operator_approvals_state_created ON operator_approvals(state, created_at DESC)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_operator_tasks_next_run ON operator_tasks(next_run_at)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_domain_policies_domain ON domain_policies(domain)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_project_artifacts_project_created ON project_artifacts(project_id, created_at DESC)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_research_jobs_state_updated ON research_jobs(state, updated_at DESC)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    fn row_to_session(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<OperatorSession> {
        let state_raw: String = row.try_get("state")?;
        Ok(OperatorSession {
            id: row.try_get("id")?,
            title: row.try_get("title")?,
            goal: row.try_get("goal")?,
            state: serde_json::from_str(&format!("\"{state_raw}\""))?,
            created_at: parse_dt(row.try_get("created_at")?)?,
            updated_at: parse_dt(row.try_get("updated_at")?)?,
        })
    }

    fn row_to_step(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<OperatorStep> {
        let state_raw: String = row.try_get("state")?;
        let trace_id: Option<String> = row.try_get("trace_id")?;
        Ok(OperatorStep {
            id: row.try_get("id")?,
            session_id: row.try_get("session_id")?,
            title: row.try_get("title")?,
            detail: row.try_get("detail")?,
            state: serde_json::from_str(&format!("\"{state_raw}\""))?,
            trace_id: trace_id.and_then(|v| Uuid::parse_str(&v).ok()),
            created_at: parse_dt(row.try_get("created_at")?)?,
        })
    }

    fn row_to_action(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<OperatorActionProposal> {
        let action_kind_raw: String = row.try_get("action_kind")?;
        let state_raw: String = row.try_get("state")?;
        let params_json: String = row.try_get("params_json")?;
        Ok(OperatorActionProposal {
            id: row.try_get("id")?,
            session_id: row.try_get("session_id")?,
            step_id: row.try_get("step_id")?,
            action_kind: serde_json::from_str(&format!("\"{action_kind_raw}\""))?,
            target: row.try_get("target")?,
            params: serde_json::from_str(&params_json).unwrap_or(Value::Null),
            state: serde_json::from_str(&format!("\"{state_raw}\""))?,
            approval_token: row.try_get("approval_token")?,
            reason: row.try_get("reason")?,
            created_at: parse_dt(row.try_get("created_at")?)?,
            updated_at: parse_dt(row.try_get("updated_at")?)?,
        })
    }

    fn row_to_task(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<OperatorTaskTemplate> {
        Ok(OperatorTaskTemplate {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            cron: row.try_get("cron")?,
            prompt: row.try_get("prompt")?,
            target_project: row.try_get("target_project")?,
            enabled: row.try_get::<i64, _>("enabled")? == 1,
            next_run_at: row
                .try_get::<Option<String>, _>("next_run_at")?
                .and_then(|v| parse_dt(v).ok()),
            last_run_at: row
                .try_get::<Option<String>, _>("last_run_at")?
                .and_then(|v| parse_dt(v).ok()),
        })
    }

    fn row_to_research_job(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<ResearchJobRecord> {
        Ok(ResearchJobRecord {
            id: row.try_get("id")?,
            project_id: row.try_get("project_id")?,
            title: row.try_get("title")?,
            question: row.try_get("question")?,
            state: row.try_get("state")?,
            source_urls: parse_json_strings(row.try_get("source_urls_json")?),
            report: row
                .try_get::<Option<String>, _>("report_json")?
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok()),
            error: row.try_get("error")?,
            created_at: parse_dt(row.try_get("created_at")?)?,
            updated_at: parse_dt(row.try_get("updated_at")?)?,
            completed_at: row
                .try_get::<Option<String>, _>("completed_at")?
                .and_then(|raw| parse_dt(raw).ok()),
        })
    }

    pub async fn create_session(&self, title: &str, goal: &str) -> anyhow::Result<OperatorSession> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO operator_sessions (id, title, goal, state, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(title)
        .bind(goal)
        .bind("draft")
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        self.get_session(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("created session not found"))
    }

    pub async fn get_session(&self, id: &str) -> anyhow::Result<Option<OperatorSession>> {
        let row = sqlx::query(
            "SELECT id, title, goal, state, created_at, updated_at FROM operator_sessions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(Self::row_to_session).transpose()
    }

    pub async fn list_sessions(&self, limit: usize) -> anyhow::Result<Vec<OperatorSession>> {
        let rows = sqlx::query(
            "SELECT id, title, goal, state, created_at, updated_at FROM operator_sessions ORDER BY updated_at DESC LIMIT ?",
        )
        .bind(limit.max(1) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_session).collect()
    }

    pub async fn set_session_state(
        &self,
        id: &str,
        state: OperatorSessionState,
    ) -> anyhow::Result<Option<OperatorSession>> {
        let now = Utc::now().to_rfc3339();
        let state_raw = serde_json::to_string(&state)?.trim_matches('"').to_string();
        let updated =
            sqlx::query("UPDATE operator_sessions SET state = ?, updated_at = ? WHERE id = ?")
                .bind(state_raw)
                .bind(now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        if updated.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_session(id).await
    }

    pub async fn replace_plan_steps(
        &self,
        session_id: &str,
        steps: &[String],
    ) -> anyhow::Result<Vec<OperatorStep>> {
        sqlx::query("DELETE FROM operator_steps WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await?;

        let now = Utc::now().to_rfc3339();
        for (idx, line) in steps.iter().enumerate() {
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                r#"
                INSERT INTO operator_steps (id, session_id, title, detail, state, trace_id, created_at)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(id)
            .bind(session_id)
            .bind(format!("step-{:02}", idx + 1))
            .bind(line)
            .bind("pending")
            .bind(Option::<String>::None)
            .bind(&now)
            .execute(&self.pool)
            .await?;
        }

        let _ = self
            .set_session_state(session_id, OperatorSessionState::Planned)
            .await?;
        self.list_steps(session_id).await
    }

    pub async fn list_steps(&self, session_id: &str) -> anyhow::Result<Vec<OperatorStep>> {
        let rows = sqlx::query(
            "SELECT id, session_id, title, detail, state, trace_id, created_at FROM operator_steps WHERE session_id = ? ORDER BY created_at ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_step).collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn propose_action(
        &self,
        session_id: &str,
        step_id: Option<String>,
        action_kind: ActionKind,
        target: Option<String>,
        params: Value,
        approval_token: Option<String>,
        reason: Option<String>,
    ) -> anyhow::Result<OperatorActionProposal> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let action_kind_raw = serde_json::to_string(&action_kind)?
            .trim_matches('"')
            .to_string();
        let params_json = serde_json::to_string(&params)?;
        sqlx::query(
            r#"
            INSERT INTO operator_action_proposals
            (id, session_id, step_id, action_kind, target, params_json, state, approval_token, reason, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(session_id)
        .bind(step_id)
        .bind(action_kind_raw)
        .bind(target)
        .bind(params_json)
        .bind("pending_approval")
        .bind(approval_token)
        .bind(reason)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        self.get_action(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("proposed action not found"))
    }

    pub async fn get_action(&self, id: &str) -> anyhow::Result<Option<OperatorActionProposal>> {
        let row = sqlx::query(
            r#"
            SELECT id, session_id, step_id, action_kind, target, params_json, state, approval_token, reason, created_at, updated_at
            FROM operator_action_proposals
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(Self::row_to_action).transpose()
    }

    pub async fn list_pending_approvals(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<OperatorActionProposal>> {
        let rows = sqlx::query(
            r#"
            SELECT id, session_id, step_id, action_kind, target, params_json, state, approval_token, reason, created_at, updated_at
            FROM operator_action_proposals
            WHERE state = 'pending_approval'
            ORDER BY created_at ASC
            LIMIT ?
            "#,
        )
        .bind(limit.max(1) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_action).collect()
    }

    pub async fn set_action_state(
        &self,
        action_id: &str,
        state: OperatorActionState,
        actor: &str,
    ) -> anyhow::Result<Option<OperatorActionProposal>> {
        let now = Utc::now().to_rfc3339();
        let state_raw = serde_json::to_string(&state)?.trim_matches('"').to_string();
        let update = sqlx::query(
            "UPDATE operator_action_proposals SET state = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&state_raw)
        .bind(&now)
        .bind(action_id)
        .execute(&self.pool)
        .await?;
        if update.rows_affected() == 0 {
            return Ok(None);
        }

        sqlx::query(
            r#"
            INSERT INTO operator_approvals (action_id, state, token, actor, created_at, updated_at)
            VALUES (?, ?, NULL, ?, ?, ?)
            "#,
        )
        .bind(action_id)
        .bind(&state_raw)
        .bind(actor)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        self.get_action(action_id).await
    }

    pub async fn save_action_result(
        &self,
        action_id: &str,
        ok: bool,
        output: Value,
        error: Option<String>,
    ) -> anyhow::Result<OperatorActionResult> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let output_json = serde_json::to_string(&output)?;
        sqlx::query(
            r#"
            INSERT INTO operator_action_results (id, action_id, ok, output_json, error, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(action_id)
        .bind(if ok { 1 } else { 0 })
        .bind(output_json)
        .bind(error.clone())
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(OperatorActionResult {
            id,
            action_id: action_id.to_string(),
            ok,
            output,
            error,
            created_at: parse_dt(now)?,
        })
    }

    pub async fn timeline(
        &self,
        session_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<OperatorTimelineItem>> {
        let mut items = Vec::new();
        if let Some(session) = self.get_session(session_id).await? {
            items.push(OperatorTimelineItem {
                at: session.created_at,
                kind: "session".into(),
                summary: format!("session created: {}", session.title),
                payload: serde_json::json!({
                    "state": session.state,
                    "goal": session.goal
                }),
            });
        }

        for step in self.list_steps(session_id).await? {
            items.push(OperatorTimelineItem {
                at: step.created_at,
                kind: "step".into(),
                summary: format!("{} ({:?})", step.title, step.state),
                payload: serde_json::to_value(step)?,
            });
        }

        let action_rows = sqlx::query(
            r#"
            SELECT id, session_id, step_id, action_kind, target, params_json, state, approval_token, reason, created_at, updated_at
            FROM operator_action_proposals
            WHERE session_id = ?
            ORDER BY created_at ASC
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        for row in &action_rows {
            let action = Self::row_to_action(row)?;
            items.push(OperatorTimelineItem {
                at: action.created_at,
                kind: "action".into(),
                summary: format!("{:?} -> {:?}", action.action_kind, action.state),
                payload: serde_json::to_value(action)?,
            });
        }

        items.sort_by(|a, b| a.at.cmp(&b.at));
        if items.len() > limit.max(1) {
            let start = items.len() - limit.max(1);
            Ok(items[start..].to_vec())
        } else {
            Ok(items)
        }
    }

    pub async fn create_task(&self, req: NewOperatorTask) -> anyhow::Result<OperatorTaskTemplate> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let next = if req.enabled {
            next_cron_time(&req.cron, now)
        } else {
            None
        };
        sqlx::query(
            r#"
            INSERT INTO operator_tasks (id, name, cron, prompt, target_project, enabled, next_run_at, last_run_at, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(req.name)
        .bind(&req.cron)
        .bind(req.prompt)
        .bind(req.target_project)
        .bind(if req.enabled { 1 } else { 0 })
        .bind(next.map(|v| v.to_rfc3339()))
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        self.get_task(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("created task not found"))
    }

    pub async fn get_task(&self, id: &str) -> anyhow::Result<Option<OperatorTaskTemplate>> {
        let row = sqlx::query(
            "SELECT id, name, cron, prompt, target_project, enabled, next_run_at, last_run_at FROM operator_tasks WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(Self::row_to_task).transpose()
    }

    pub async fn list_tasks(&self, limit: usize) -> anyhow::Result<Vec<OperatorTaskTemplate>> {
        let rows = sqlx::query(
            "SELECT id, name, cron, prompt, target_project, enabled, next_run_at, last_run_at FROM operator_tasks ORDER BY COALESCE(next_run_at, '9999') ASC LIMIT ?",
        )
        .bind(limit.max(1) as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_task).collect()
    }

    pub async fn run_task_now(
        &self,
        task_id: &str,
    ) -> anyhow::Result<Option<OperatorTaskTemplate>> {
        let Some(task) = self.get_task(task_id).await? else {
            return Ok(None);
        };
        let now = Utc::now();
        let next = if task.enabled {
            next_cron_time(&task.cron, now)
        } else {
            None
        };
        sqlx::query("UPDATE operator_tasks SET last_run_at = ?, next_run_at = ?, updated_at = ? WHERE id = ?")
            .bind(now.to_rfc3339())
            .bind(next.map(|v| v.to_rfc3339()))
            .bind(now.to_rfc3339())
            .bind(task_id)
            .execute(&self.pool)
            .await?;
        self.get_task(task_id).await
    }

    pub async fn poll_due_tasks(
        &self,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Vec<OperatorTaskTemplate>> {
        let rows = sqlx::query(
            "SELECT id, name, cron, prompt, target_project, enabled, next_run_at, last_run_at FROM operator_tasks WHERE enabled = 1",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut due = Vec::new();
        for row in &rows {
            let task = Self::row_to_task(row)?;
            if let Some(next) = task.next_run_at {
                if next <= now {
                    due.push(task);
                }
            }
        }
        Ok(due)
    }

    pub async fn set_domain_policy(
        &self,
        domain: String,
        profile: ApprovalProfile,
        blocked_actions: Vec<ActionKind>,
        allow_actions: Vec<ActionKind>,
    ) -> anyhow::Result<DomainPolicy> {
        let now = Utc::now();
        let profile_raw = serde_json::to_string(&profile)?
            .trim_matches('"')
            .to_string();
        let blocked_actions_json = serde_json::to_string(&blocked_actions)?;
        let allow_actions_json = serde_json::to_string(&allow_actions)?;
        sqlx::query(
            r#"
            INSERT INTO domain_policies(domain, profile, blocked_actions_json, allow_actions_json, updated_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(domain) DO UPDATE SET
                profile = excluded.profile,
                blocked_actions_json = excluded.blocked_actions_json,
                allow_actions_json = excluded.allow_actions_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&domain)
        .bind(profile_raw)
        .bind(blocked_actions_json)
        .bind(allow_actions_json)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        self.get_domain_policy(&domain)
            .await?
            .ok_or_else(|| anyhow::anyhow!("saved policy not found"))
    }

    pub async fn list_domain_policies(&self) -> anyhow::Result<Vec<DomainPolicy>> {
        let rows = sqlx::query(
            "SELECT domain, profile, blocked_actions_json, allow_actions_json, updated_at FROM domain_policies ORDER BY domain ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let profile_raw: String = row.try_get("profile")?;
            let blocked_actions_raw: String = row.try_get("blocked_actions_json")?;
            let allow_actions_raw: String = row.try_get("allow_actions_json")?;
            out.push(DomainPolicy {
                domain: row.try_get("domain")?,
                profile: serde_json::from_str(&format!("\"{profile_raw}\""))?,
                blocked_actions: serde_json::from_str(&blocked_actions_raw).unwrap_or_default(),
                allow_actions: serde_json::from_str(&allow_actions_raw).unwrap_or_default(),
                updated_at: parse_dt(row.try_get("updated_at")?)?,
            });
        }
        Ok(out)
    }

    pub async fn get_domain_policy(&self, domain: &str) -> anyhow::Result<Option<DomainPolicy>> {
        let row = sqlx::query(
            "SELECT domain, profile, blocked_actions_json, allow_actions_json, updated_at FROM domain_policies WHERE domain = ?",
        )
        .bind(domain)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let profile_raw: String = row.try_get("profile")?;
        let blocked_actions_raw: String = row.try_get("blocked_actions_json")?;
        let allow_actions_raw: String = row.try_get("allow_actions_json")?;
        Ok(Some(DomainPolicy {
            domain: row.try_get("domain")?,
            profile: serde_json::from_str(&format!("\"{profile_raw}\""))?,
            blocked_actions: serde_json::from_str(&blocked_actions_raw).unwrap_or_default(),
            allow_actions: serde_json::from_str(&allow_actions_raw).unwrap_or_default(),
            updated_at: parse_dt(row.try_get("updated_at")?)?,
        }))
    }

    pub async fn oauth_start(
        &self,
        provider: &str,
        redirect_uri: Option<String>,
        scopes: Option<Vec<String>>,
    ) -> anyhow::Result<String> {
        let now = Utc::now().to_rfc3339();
        let scopes_json = serde_json::to_string(&scopes.unwrap_or_default())?;
        sqlx::query(
            r#"
            INSERT INTO connector_accounts(provider, connected, account_id, access_token, refresh_token, scopes_json, last_error, updated_at)
            VALUES (?, 0, NULL, NULL, NULL, ?, NULL, ?)
            ON CONFLICT(provider) DO UPDATE SET
                scopes_json = excluded.scopes_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(provider)
        .bind(scopes_json)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        let redirect = redirect_uri.unwrap_or_else(|| "http://localhost/callback".into());
        Ok(format!(
            "https://auth.clawork.local/{provider}/start?redirect_uri={redirect}"
        ))
    }

    pub async fn oauth_callback(
        &self,
        provider: &str,
        payload: OAuthCallbackInput,
    ) -> anyhow::Result<ConnectorStatus> {
        let now = Utc::now().to_rfc3339();
        let scopes_json = serde_json::to_string(&payload.scopes.unwrap_or_default())?;
        let connected = if payload.error.is_none() { 1 } else { 0 };

        sqlx::query(
            r#"
            INSERT INTO connector_accounts(provider, connected, account_id, access_token, refresh_token, scopes_json, last_error, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(provider) DO UPDATE SET
                connected = excluded.connected,
                account_id = excluded.account_id,
                access_token = excluded.access_token,
                refresh_token = excluded.refresh_token,
                scopes_json = excluded.scopes_json,
                last_error = excluded.last_error,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(provider)
        .bind(connected)
        .bind(payload.account_id)
        .bind(payload.access_token)
        .bind(payload.refresh_token)
        .bind(scopes_json)
        .bind(payload.error)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        self.connector_status(provider)
            .await?
            .ok_or_else(|| anyhow::anyhow!("connector status not found"))
    }

    pub async fn connector_status(
        &self,
        provider: &str,
    ) -> anyhow::Result<Option<ConnectorStatus>> {
        let row = sqlx::query(
            "SELECT provider, connected, account_id, last_error, updated_at FROM connector_accounts WHERE provider = ?",
        )
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(ConnectorStatus {
            provider: row.try_get("provider")?,
            connected: row.try_get::<i64, _>("connected")? == 1,
            account_id: row.try_get("account_id")?,
            last_error: row.try_get("last_error")?,
            updated_at: parse_dt(row.try_get("updated_at")?)?,
        }))
    }

    pub async fn connector_statuses(&self) -> anyhow::Result<Vec<ConnectorStatus>> {
        let rows = sqlx::query(
            "SELECT provider, connected, account_id, last_error, updated_at FROM connector_accounts ORDER BY provider ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(ConnectorStatus {
                provider: row.try_get("provider")?,
                connected: row.try_get::<i64, _>("connected")? == 1,
                account_id: row.try_get("account_id")?,
                last_error: row.try_get("last_error")?,
                updated_at: parse_dt(row.try_get("updated_at")?)?,
            });
        }
        Ok(out)
    }

    pub async fn connector_access_token(&self, provider: &str) -> anyhow::Result<Option<String>> {
        let row = sqlx::query(
            "SELECT connected, access_token FROM connector_accounts WHERE provider = ?",
        )
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };

        let connected: i64 = row.try_get("connected")?;
        if connected != 1 {
            return Ok(None);
        }

        let token: Option<String> = row.try_get("access_token")?;
        Ok(token.and_then(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }))
    }

    pub async fn create_project(
        &self,
        name: &str,
        description: Option<String>,
    ) -> anyhow::Result<ProjectInfo> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let desc_clone = description.clone();
        sqlx::query("INSERT INTO projects (id, name, description, created_at) VALUES (?, ?, ?, ?)")
            .bind(&id)
            .bind(name)
            .bind(description)
            .bind(&now)
            .execute(&self.pool)
            .await?;
        Ok(ProjectInfo {
            id,
            name: name.to_string(),
            description: desc_clone,
            created_at: parse_dt(now)?,
        })
    }

    pub async fn list_projects(&self, limit: usize) -> anyhow::Result<Vec<ProjectInfo>> {
        let rows = sqlx::query(
            "SELECT id, name, description, created_at FROM projects ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit.max(1) as i64)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(ProjectInfo {
                id: row.try_get("id")?,
                name: row.try_get("name")?,
                description: row.try_get("description")?,
                created_at: parse_dt(row.try_get("created_at")?)?,
            });
        }
        Ok(out)
    }

    pub async fn create_research_job(
        &self,
        project_id: Option<String>,
        title: String,
        question: String,
        source_urls: Vec<String>,
    ) -> anyhow::Result<ResearchJobRecord> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let source_urls_json = serde_json::to_string(&source_urls)?;
        sqlx::query(
            r#"
            INSERT INTO research_jobs(id, project_id, title, question, state, source_urls_json, report_json, error, created_at, updated_at, completed_at)
            VALUES (?, ?, ?, ?, 'running', ?, NULL, NULL, ?, ?, NULL)
            "#,
        )
        .bind(&id)
        .bind(project_id)
        .bind(title)
        .bind(question)
        .bind(source_urls_json)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        self.get_research_job(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("created research job not found"))
    }

    pub async fn complete_research_job(
        &self,
        id: &str,
        report: Value,
    ) -> anyhow::Result<Option<ResearchJobRecord>> {
        let now = Utc::now().to_rfc3339();
        let report_json = serde_json::to_string(&report)?;
        let updated = sqlx::query(
            "UPDATE research_jobs SET state = 'completed', report_json = ?, error = NULL, updated_at = ?, completed_at = ? WHERE id = ?",
        )
        .bind(report_json)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_research_job(id).await
    }

    pub async fn fail_research_job(
        &self,
        id: &str,
        error: String,
    ) -> anyhow::Result<Option<ResearchJobRecord>> {
        let now = Utc::now().to_rfc3339();
        let updated = sqlx::query(
            "UPDATE research_jobs SET state = 'failed', error = ?, updated_at = ?, completed_at = ? WHERE id = ?",
        )
        .bind(error)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_research_job(id).await
    }

    pub async fn get_research_job(&self, id: &str) -> anyhow::Result<Option<ResearchJobRecord>> {
        let row = sqlx::query(
            "SELECT id, project_id, title, question, state, source_urls_json, report_json, error, created_at, updated_at, completed_at FROM research_jobs WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(Self::row_to_research_job).transpose()
    }

    pub async fn add_artifact(
        &self,
        project_id: &str,
        path: String,
        mime: String,
        producer_step: Option<String>,
        citations: Vec<CitationRef>,
    ) -> anyhow::Result<ProjectArtifact> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let citations_json = serde_json::to_string(&citations)?;
        sqlx::query(
            r#"
            INSERT INTO project_artifacts(id, project_id, path, mime, producer_step, citations_json, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(project_id)
        .bind(&path)
        .bind(&mime)
        .bind(producer_step.clone())
        .bind(citations_json)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(ProjectArtifact {
            id,
            project_id: project_id.to_string(),
            path,
            mime,
            producer_step,
            citations,
            created_at: parse_dt(now)?,
        })
    }

    pub async fn list_artifacts(
        &self,
        project_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ProjectArtifact>> {
        let rows = sqlx::query(
            "SELECT id, project_id, path, mime, producer_step, citations_json, created_at FROM project_artifacts WHERE project_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(project_id)
        .bind(limit.max(1) as i64)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(ProjectArtifact {
                id: row.try_get("id")?,
                project_id: row.try_get("project_id")?,
                path: row.try_get("path")?,
                mime: row.try_get("mime")?,
                producer_step: row.try_get("producer_step")?,
                citations: parse_json_citations(row.try_get("citations_json")?),
                created_at: parse_dt(row.try_get("created_at")?)?,
            });
        }
        Ok(out)
    }

    pub async fn operator_status_counts(
        &self,
    ) -> anyhow::Result<(usize, usize, usize, BTreeMap<String, bool>)> {
        let active_sessions: i64 = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM operator_sessions WHERE state IN ('draft', 'planned', 'running', 'waiting_approval')",
        )
        .fetch_one(&self.pool)
        .await?
        .try_get("cnt")?;
        let pending_approvals: i64 = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM operator_action_proposals WHERE state = 'pending_approval'",
        )
        .fetch_one(&self.pool)
        .await?
        .try_get("cnt")?;
        let scheduled_tasks: i64 =
            sqlx::query("SELECT COUNT(*) AS cnt FROM operator_tasks WHERE enabled = 1")
                .fetch_one(&self.pool)
                .await?
                .try_get("cnt")?;

        let statuses = self.connector_statuses().await?;
        let mut map = BTreeMap::new();
        for status in statuses {
            map.insert(status.provider, status.connected);
        }

        Ok((
            active_sessions.max(0) as usize,
            pending_approvals.max(0) as usize,
            scheduled_tasks.max(0) as usize,
            map,
        ))
    }
}

fn parse_dt(value: String) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(&value)?.with_timezone(&Utc))
}

fn parse_json_citations(value: Option<String>) -> Vec<CitationRef> {
    value
        .and_then(|raw| serde_json::from_str::<Vec<CitationRef>>(&raw).ok())
        .unwrap_or_default()
}

fn parse_json_strings(value: Option<String>) -> Vec<String> {
    value
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
}

fn next_cron_time(expr: &str, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let schedule = Schedule::from_str(expr).ok()?;
    schedule.upcoming(Utc).find(|dt| *dt > from)
}
