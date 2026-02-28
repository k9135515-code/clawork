use async_trait::async_trait;
use chrono::{Duration, Utc};
use clawork_core::{
    ActionKind, ActionRequest, CapabilityGuard, Decision, PermissionMode, RequestContext,
};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct PendingConfirmation {
    expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone)]
pub struct PermissionEngine {
    mode: Arc<RwLock<PermissionMode>>,
    pending: Arc<RwLock<HashMap<String, PendingConfirmation>>>,
    approved: Arc<RwLock<HashMap<String, chrono::DateTime<chrono::Utc>>>>,
}

#[derive(Clone, Debug)]
pub struct AllowedPathPolicy {
    roots: Arc<Vec<PathBuf>>,
}

impl Default for PermissionEngine {
    fn default() -> Self {
        Self {
            mode: Arc::new(RwLock::new(PermissionMode::Sandbox)),
            pending: Arc::new(RwLock::new(HashMap::new())),
            approved: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl PermissionEngine {
    pub fn current_mode(&self) -> PermissionMode {
        let _ = self.expire_elevation_if_needed();
        self.mode.read().clone()
    }

    pub fn request_elevation(&self, ttl_seconds: i64) -> PermissionMode {
        let expires_at = Utc::now() + Duration::seconds(ttl_seconds.max(30));
        let mode = PermissionMode::Elevated { expires_at };
        *self.mode.write() = mode.clone();
        mode
    }

    pub fn approve_token(&self, token: &str) -> bool {
        let _ = self.expire_elevation_if_needed();
        let now = Utc::now();
        let pending = self.pending.read();
        if let Some(entry) = pending.get(token) {
            if entry.expires_at > now {
                self.approved
                    .write()
                    .insert(token.to_string(), entry.expires_at);
                return true;
            }
        }
        false
    }

    pub fn reset_to_sandbox(&self) {
        *self.mode.write() = PermissionMode::Sandbox;
        self.pending.write().clear();
        self.approved.write().clear();
    }

    pub fn expire_elevation_if_needed(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        let now = Utc::now();
        let mut mode_guard = self.mode.write();
        if let PermissionMode::Elevated { expires_at } = &*mode_guard {
            if *expires_at <= now {
                let expired_at = expires_at.to_owned();
                *mode_guard = PermissionMode::Sandbox;
                drop(mode_guard);
                self.pending.write().clear();
                self.approved.write().clear();
                return Some(expired_at);
            }
        }
        None
    }

    fn needs_confirmation(action: &ActionKind) -> bool {
        matches!(
            action,
            ActionKind::FileWrite
                | ActionKind::ShellExec
                | ActionKind::BrowserNav
                | ActionKind::NetworkCall
                | ActionKind::MessageSend
                | ActionKind::SkillInstall
                | ActionKind::ElevatedRequest
        )
    }
}

impl AllowedPathPolicy {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots: Arc::new(roots),
        }
    }

    pub fn roots(&self) -> &[PathBuf] {
        self.roots.as_ref()
    }

    pub fn is_allowed(&self, path: &Path) -> bool {
        let Some(target) = canonicalish(path) else {
            return false;
        };
        self.roots.iter().any(|root| {
            canonicalish(root)
                .map(|r| target.starts_with(r))
                .unwrap_or(false)
        })
    }

    pub fn assert_allowed(&self, path: &Path) -> anyhow::Result<()> {
        if self.is_allowed(path) {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "path is outside sandbox roots: {}",
                path.display()
            ))
        }
    }
}

fn canonicalish(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        path.canonicalize().ok()
    } else {
        let mut ancestors = path.ancestors();
        let mut existing = None;
        for anc in &mut ancestors {
            if anc.exists() {
                existing = Some(anc);
                break;
            }
        }
        let existing = existing?;
        let canonical_existing = existing.canonicalize().ok()?;
        let relative = path.strip_prefix(existing).ok()?;
        Some(canonical_existing.join(relative))
    }
}

#[async_trait]
impl CapabilityGuard for PermissionEngine {
    async fn authorize(&self, action: &ActionRequest, ctx: &RequestContext) -> Decision {
        let now = Utc::now();

        let _ = self.expire_elevation_if_needed();
        let current_mode = self.current_mode();

        if !Self::needs_confirmation(&action.kind) {
            return Decision::Allow;
        }

        if current_mode.is_elevated_active(now) {
            return Decision::Allow;
        }

        if let Some(token) = &ctx.approval_token {
            let approved = self.approved.read();
            if let Some(exp) = approved.get(token) {
                if *exp > now {
                    return Decision::Allow;
                }
            }
        }

        let token = format!("confirm-{}", Uuid::new_v4());
        self.pending.write().insert(
            token.clone(),
            PendingConfirmation {
                expires_at: now + Duration::minutes(5),
            },
        );

        Decision::RequiresConfirmation {
            token,
            reason: "Dangerous action requires explicit user confirmation".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawork_core::{ActionRequest, PermissionMode, RequestContext};
    use serde_json::json;

    #[tokio::test]
    async fn confirms_shell_exec_in_sandbox() {
        let guard = PermissionEngine::default();
        let action = ActionRequest {
            kind: ActionKind::ShellExec,
            target: Some("cmd.exe".into()),
            params: json!({}),
            trace_id: Uuid::new_v4(),
        };
        let ctx = RequestContext {
            actor: "tester".into(),
            mode: PermissionMode::Sandbox,
            approval_token: None,
        };

        let decision = guard.authorize(&action, &ctx).await;
        assert!(matches!(decision, Decision::RequiresConfirmation { .. }));
    }

    #[tokio::test]
    async fn confirms_message_send_in_sandbox() {
        let guard = PermissionEngine::default();
        let action = ActionRequest {
            kind: ActionKind::MessageSend,
            target: Some("telegram:123456".into()),
            params: json!({"content_len": 4}),
            trace_id: Uuid::new_v4(),
        };
        let ctx = RequestContext {
            actor: "tester".into(),
            mode: PermissionMode::Sandbox,
            approval_token: None,
        };

        let decision = guard.authorize(&action, &ctx).await;
        assert!(matches!(decision, Decision::RequiresConfirmation { .. }));
    }

    #[test]
    fn elevated_expiry_forces_sandbox_and_clears_tokens() {
        let guard = PermissionEngine::default();
        {
            let mut mode = guard.mode.write();
            *mode = PermissionMode::Elevated {
                expires_at: Utc::now() - Duration::seconds(1),
            };
        }
        guard.pending.write().insert(
            "confirm-old".into(),
            PendingConfirmation {
                expires_at: Utc::now() + Duration::minutes(5),
            },
        );
        guard
            .approved
            .write()
            .insert("confirm-old".into(), Utc::now() + Duration::minutes(5));

        let expired = guard.expire_elevation_if_needed();
        assert!(expired.is_some());
        assert!(matches!(guard.current_mode(), PermissionMode::Sandbox));
        assert!(guard.pending.read().is_empty());
        assert!(guard.approved.read().is_empty());
    }
}
