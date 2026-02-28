use anyhow::Context;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Clone)]
pub struct MemoryStore {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryHit {
    pub record: MemoryRecord,
    pub score: f32,
}

impl MemoryStore {
    pub async fn connect(database_url: &str) -> anyhow::Result<Self> {
        let pool = SqlitePool::connect(database_url)
            .await
            .with_context(|| format!("connect memory db: {database_url}"))?;

        let store = Self { pool };
        store.init().await?;
        Ok(store)
    }

    async fn init(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                text TEXT NOT NULL,
                embedding_json TEXT
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_memories_created_at ON memories(created_at DESC)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn store_text(
        &self,
        text: &str,
        embedding: Option<&[f32]>,
    ) -> anyhow::Result<String> {
        let id = Uuid::new_v4().to_string();
        let embedding_json = embedding.map(serde_json::to_string).transpose()?;

        sqlx::query(
            r#"
            INSERT INTO memories (id, created_at, text, embedding_json)
            VALUES (?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(Utc::now().to_rfc3339())
        .bind(text)
        .bind(embedding_json)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    pub async fn count(&self) -> anyhow::Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) as cnt FROM memories")
            .fetch_one(&self.pool)
            .await?;
        let cnt: i64 = row.try_get("cnt")?;
        Ok(cnt)
    }

    pub async fn recent(&self, limit: i64) -> anyhow::Result<Vec<MemoryRecord>> {
        let rows = sqlx::query(
            "SELECT id, created_at, text FROM memories ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await?;

        rows_to_records(rows)
    }

    pub async fn search_like(&self, query: &str, limit: i64) -> anyhow::Result<Vec<MemoryRecord>> {
        let pattern = format!("%{query}%");
        let rows = sqlx::query(
            "SELECT id, created_at, text FROM memories WHERE text LIKE ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(pattern)
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await?;

        rows_to_records(rows)
    }

    pub async fn search_by_embedding(
        &self,
        query_embedding: &[f32],
        limit: i64,
    ) -> anyhow::Result<Vec<MemoryHit>> {
        let rows = sqlx::query(
            "SELECT id, created_at, text, embedding_json FROM memories WHERE embedding_json IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut hits = Vec::new();
        for row in rows {
            let embedding_raw: Option<String> = row.try_get("embedding_json")?;
            let Some(embedding_raw) = embedding_raw else {
                continue;
            };
            let Ok(embedding_vec) = serde_json::from_str::<Vec<f32>>(&embedding_raw) else {
                continue;
            };

            let score = cosine_similarity(query_embedding, &embedding_vec);
            let record = MemoryRecord {
                id: row.try_get("id")?,
                created_at: chrono::DateTime::parse_from_rfc3339(
                    &row.try_get::<String, _>("created_at")?,
                )?
                .with_timezone(&chrono::Utc),
                text: row.try_get("text")?,
            };
            hits.push(MemoryHit { record, score });
        }

        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(limit.max(1) as usize);
        Ok(hits)
    }
}

fn rows_to_records(rows: Vec<sqlx::sqlite::SqliteRow>) -> anyhow::Result<Vec<MemoryRecord>> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(MemoryRecord {
            id: row.try_get("id")?,
            created_at: chrono::DateTime::parse_from_rfc3339(
                &row.try_get::<String, _>("created_at")?,
            )?
            .with_timezone(&chrono::Utc),
            text: row.try_get("text")?,
        });
    }

    Ok(out)
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let n = a.len().min(b.len());
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for i in 0..n {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    if norm_a <= f32::EPSILON || norm_b <= f32::EPSILON {
        0.0
    } else {
        dot / (norm_a.sqrt() * norm_b.sqrt())
    }
}
