use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct WorkspaceGuard {
    allowed_roots: Vec<PathBuf>,
    read_only: bool,
}

impl WorkspaceGuard {
    pub fn new(allowed_paths: Vec<PathBuf>, read_only: bool) -> Result<Self> {
        let mut allowed_roots = Vec::new();
        for path in allowed_paths {
            let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            allowed_roots.push(canonical);
        }
        Ok(Self {
            allowed_roots,
            read_only,
        })
    }

    #[allow(dead_code)]
    pub fn validate_path<P: AsRef<Path>>(&self, target: P, is_write: bool) -> Result<PathBuf> {
        if is_write && self.read_only {
            return Err(anyhow!(
                "File editing is disabled because the workspace is in Read-Only mode"
            ));
        }

        let target_path = target.as_ref();
        let canonical_target = if target_path.exists() {
            std::fs::canonicalize(target_path)?
        } else if let Some(parent) = target_path.parent() {
            let canonical_parent = std::fs::canonicalize(parent)?;
            canonical_parent.join(
                target_path
                    .file_name()
                    .ok_or_else(|| anyhow!("Invalid file name"))?,
            )
        } else {
            target_path.to_path_buf()
        };

        let is_allowed = self
            .allowed_roots
            .iter()
            .any(|root| canonical_target.starts_with(root));

        if !is_allowed {
            return Err(anyhow!(
                "File access denied: path {:?} is outside the allowed scope",
                target_path
            ));
        }

        Ok(canonical_target)
    }
}
