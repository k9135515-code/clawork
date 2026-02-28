use anyhow::Context;
use clawork_core::AuditEvent;
use sqlx::{Row, SqlitePool};

#[derive(Clone)]
pub struct AuditStore {
    pool: SqlitePool,
}

impl AuditStore {
    pub async fn connect(database_url: &str) -> anyhow::Result<Self> {
        let pool = SqlitePool::connect(database_url)
            .await
            .with_context(|| format!("connect to audit db: {database_url}"))?;
        let store = Self { pool };
        store.init().await?;
        Ok(store)
    }

    async fn init(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                actor TEXT NOT NULL,
                action TEXT NOT NULL,
                target TEXT,
                decision TEXT NOT NULL,
                reason TEXT,
                trace_id TEXT NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_audit_events_timestamp ON audit_events(timestamp);",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn append(&self, event: &AuditEvent) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO audit_events (timestamp, actor, action, target, decision, reason, trace_id)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(event.timestamp.to_rfc3339())
        .bind(&event.actor)
        .bind(serde_json::to_string(&event.action)?)
        .bind(&event.target)
        .bind(&event.decision)
        .bind(&event.reason)
        .bind(event.trace_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_recent(&self, limit: i64) -> anyhow::Result<Vec<AuditEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT timestamp, actor, action, target, decision, reason, trace_id
            FROM audit_events
            ORDER BY id DESC
            LIMIT ?
            "#,
        )
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let action_raw: String = row.try_get("action")?;
            let event = AuditEvent {
                timestamp: chrono::DateTime::parse_from_rfc3339(
                    &row.try_get::<String, _>("timestamp")?,
                )?
                .with_timezone(&chrono::Utc),
                actor: row.try_get("actor")?,
                action: serde_json::from_str(&action_raw)?,
                target: row.try_get("target")?,
                decision: row.try_get("decision")?,
                reason: row.try_get("reason")?,
                trace_id: row.try_get::<String, _>("trace_id")?.parse()?,
            };
            out.push(event);
        }

        Ok(out)
    }

    pub async fn rotate(&self, retention_days: i64, max_rows: i64) -> anyhow::Result<()> {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days.max(1));

        sqlx::query("DELETE FROM audit_events WHERE timestamp < ?")
            .bind(cutoff.to_rfc3339())
            .execute(&self.pool)
            .await?;

        if max_rows > 0 {
            sqlx::query(
                r#"
                DELETE FROM audit_events
                WHERE id NOT IN (
                    SELECT id FROM audit_events ORDER BY id DESC LIMIT ?
                )
                "#,
            )
            .bind(max_rows)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }
}
