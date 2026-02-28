use anyhow::Context;
use async_trait::async_trait;
use chrono::Utc;
use clawork_capabilities::AllowedPathPolicy;
use clawork_core::{
    FilesystemService, FsEntry, FsOperationKind, FsOperationRequest, FsOperationResult,
    PermissionMode,
};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct SandboxFsService {
    policy: AllowedPathPolicy,
    max_read_bytes: usize,
}

impl SandboxFsService {
    pub fn new(allowed_roots: Vec<PathBuf>) -> Self {
        Self {
            policy: AllowedPathPolicy::new(allowed_roots),
            max_read_bytes: 2 * 1024 * 1024,
        }
    }

    fn ensure_path_allowed(&self, path: &Path, mode: &PermissionMode) -> anyhow::Result<()> {
        if mode.is_elevated_active(Utc::now()) {
            return Ok(());
        }
        self.policy.assert_allowed(path)
    }

    pub fn allowed_roots(&self) -> &[PathBuf] {
        self.policy.roots()
    }
}

#[async_trait]
impl FilesystemService for SandboxFsService {
    async fn operate(
        &self,
        op: FsOperationRequest,
        mode: PermissionMode,
    ) -> anyhow::Result<FsOperationResult> {
        let path = PathBuf::from(&op.path);
        self.ensure_path_allowed(&path, &mode)?;

        match op.kind {
            FsOperationKind::ReadFile => {
                let bytes = tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("read file: {}", path.display()))?;
                if bytes.len() > self.max_read_bytes {
                    return Err(anyhow::anyhow!(
                        "read exceeds max size ({} bytes): {}",
                        self.max_read_bytes,
                        path.display()
                    ));
                }

                let content = String::from_utf8_lossy(&bytes).to_string();
                Ok(FsOperationResult {
                    ok: true,
                    message: "file read".into(),
                    content: Some(content),
                    entries: vec![],
                })
            }
            FsOperationKind::WriteFile => {
                let content = op.content.unwrap_or_default();
                if let Some(parent) = path.parent() {
                    self.ensure_path_allowed(parent, &mode)?;
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(&path, content.as_bytes())
                    .await
                    .with_context(|| format!("write file: {}", path.display()))?;
                Ok(FsOperationResult {
                    ok: true,
                    message: "file written".into(),
                    content: None,
                    entries: vec![],
                })
            }
            FsOperationKind::DeleteFile => {
                tokio::fs::remove_file(&path)
                    .await
                    .with_context(|| format!("delete file: {}", path.display()))?;
                Ok(FsOperationResult {
                    ok: true,
                    message: "file deleted".into(),
                    content: None,
                    entries: vec![],
                })
            }
            FsOperationKind::ListDirectory => {
                let mut dir = tokio::fs::read_dir(&path)
                    .await
                    .with_context(|| format!("list dir: {}", path.display()))?;
                let mut entries = Vec::new();
                while let Some(item) = dir.next_entry().await? {
                    let meta = item.metadata().await?;
                    entries.push(FsEntry {
                        path: item.path().display().to_string(),
                        is_dir: meta.is_dir(),
                        size: if meta.is_file() {
                            Some(meta.len())
                        } else {
                            None
                        },
                    });
                    if entries.len() >= 200 {
                        break;
                    }
                }
                Ok(FsOperationResult {
                    ok: true,
                    message: "directory listed".into(),
                    content: None,
                    entries,
                })
            }
            FsOperationKind::CreateDirectory => {
                tokio::fs::create_dir_all(&path)
                    .await
                    .with_context(|| format!("create directory: {}", path.display()))?;
                Ok(FsOperationResult {
                    ok: true,
                    message: "directory created".into(),
                    content: None,
                    entries: vec![],
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[tokio::test]
    async fn sandbox_denies_outside_allowed_root() {
        let temp = std::env::temp_dir();
        let root = temp.join("clawork-fs-test-root");
        let outside = temp.join("clawork-fs-test-outside.txt");
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::create_dir_all(&root);

        let service = SandboxFsService::new(vec![root.clone()]);
        let op = FsOperationRequest {
            kind: FsOperationKind::WriteFile,
            path: outside.display().to_string(),
            content: Some("blocked".into()),
        };

        let res = service.operate(op, PermissionMode::Sandbox).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn elevated_allows_outside_allowed_root() {
        let temp = std::env::temp_dir();
        let root = temp.join("clawork-fs-test-root-elev");
        let outside = temp.join("clawork-fs-test-outside-elev.txt");
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::create_dir_all(&root);

        let service = SandboxFsService::new(vec![root]);
        let op = FsOperationRequest {
            kind: FsOperationKind::WriteFile,
            path: outside.display().to_string(),
            content: Some("allowed".into()),
        };

        let mode = PermissionMode::Elevated {
            expires_at: Utc::now() + Duration::seconds(60),
        };
        let res = service.operate(op, mode).await;
        assert!(res.is_ok());
        let written = std::fs::read_to_string(&outside).unwrap_or_default();
        assert_eq!(written, "allowed");
        let _ = std::fs::remove_file(&outside);
    }
}
