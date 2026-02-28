use crate::{AppState, CommandError};
use axum::http::HeaderMap;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub(crate) struct AuthTokenIssueReq {
    pub(crate) ttl_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuthTokenRevokeReq {
    pub(crate) token: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuthTokenRotateReq {
    pub(crate) ttl_seconds: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AuthTokenResult {
    pub(crate) token: String,
    pub(crate) expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub(crate) persisted: bool,
}

pub(crate) fn require_auth(state: &AppState, headers: &HeaderMap) -> Result<(), CommandError> {
    if is_authorized(state, headers) {
        Ok(())
    } else {
        Err(CommandError::denied("missing or invalid X-Clawork-Token"))
    }
}

pub(crate) fn issue_local_api_token(
    state: &AppState,
    ttl_seconds: Option<i64>,
    persist: bool,
) -> Result<AuthTokenResult, CommandError> {
    let expires_at = parse_token_ttl_seconds(ttl_seconds)?;
    let token = format!("clawork-{}", Uuid::new_v4());
    state
        .local_api_tokens
        .write()
        .insert(token.clone(), expires_at);
    if persist {
        persist_cli_token(&token)
            .map_err(|e| CommandError::internal(format!("persist token failed: {e}")))?;
    }
    Ok(AuthTokenResult {
        token,
        expires_at,
        persisted: persist,
    })
}

pub(crate) fn revoke_local_api_token(state: &AppState, token: &str) -> bool {
    state.local_api_tokens.write().remove(token).is_some()
}

pub(crate) fn ensure_cli_token() -> anyhow::Result<String> {
    let _ = std::fs::create_dir_all("data");
    let path = cli_token_path();
    if path.exists() {
        let current = std::fs::read_to_string(&path)?.trim().to_string();
        if !current.is_empty() {
            return Ok(current);
        }
    }

    let token = format!("clawork-{}", Uuid::new_v4());
    persist_cli_token(&token)?;
    Ok(token)
}

fn is_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(token) = headers.get("x-clawork-token") else {
        return false;
    };
    let Ok(token) = token.to_str() else {
        return false;
    };
    let now = Utc::now();
    let mut tokens = state.local_api_tokens.write();
    if let Some(expires_at) = tokens.get(token).cloned() {
        if let Some(expires_at) = expires_at {
            if expires_at <= now {
                tokens.remove(token);
                return false;
            }
        }
        true
    } else {
        false
    }
}

fn parse_token_ttl_seconds(
    ttl_seconds: Option<i64>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, CommandError> {
    match ttl_seconds {
        None => Ok(None),
        Some(v) if v <= 0 => Err(CommandError::validation("ttl_seconds must be > 0")),
        Some(v) => Ok(Some(Utc::now() + chrono::Duration::seconds(v))),
    }
}

fn cli_token_path() -> PathBuf {
    PathBuf::from("data/cli.token")
}

fn persist_cli_token(token: &str) -> anyhow::Result<()> {
    let _ = std::fs::create_dir_all("data");
    let path = cli_token_path();
    std::fs::write(&path, token)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms)?;
    }

    Ok(())
}
