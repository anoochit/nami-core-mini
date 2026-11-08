use anyhow::{Context, Result, anyhow};
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
            // ถ้า root ที่ประกาศยังไม่มีบนดิสก์ ให้สร้างก่อน (เช่น ./wiki/)
            // เพื่อให้ canonicalize ได้สำเร็จและได้ path สัมบูรณ์เสมอ
            if !path.exists() {
                std::fs::create_dir_all(&path)
                    .with_context(|| format!("Failed to create allowed workspace root: {:?}", path))?;
            }
            // canonicalize ต้องสำเร็จเท่านั้น: root แบบสัมพัทธ์ (เช่น "./") เมื่อเทียบกับ
            // path เป้าหมายแบบสัมบูรณ์ด้วย starts_with() จะล้มเหลวเสมอ (access denied ทั้งหมด)
            // เดิม fallback เป็น raw PathBuf ทำให้เกิดช่องโหว่นี้ — จึงต้อง fail fast เสมอ
            let canonical = std::fs::canonicalize(&path)
                .with_context(|| format!("Failed to canonicalize workspace root: {:?}", path))?;
            tracing::debug!("Workspace root allowed: {:?}", canonical);
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
            // เป้าหมายที่ไม่มี parent และไม่มีบนดิสก์: resolve จาก cwd สัมพันธ์กับ
            // เพื่อให้ผลลัพธ์เป็น absolute path ตรงกับรูปแบบของ allowed_roots เสมอ
            std::env::current_dir()?.join(target_path)
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

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: relative root (เช่น "./") ต้องถูก canonicalize เป็น absolute path ณ init
    // มิฉะนั้น starts_with() จะเทียบ absolute target กับ root แบบสัมพัทธ์แล้ว fail ทั้งหมด
    #[test]
    fn test_allowed_roots_are_canonicalized_to_absolute() {
        let guard = WorkspaceGuard::new(vec![PathBuf::from("./")], false).unwrap();
        for root in guard.allowed_roots {
            assert!(root.is_absolute(), "root must be absolute: {:?}", root);
        }
    }

    // Regression: root ที่ประกาศไว้แต่ยังไม่มีบนดิสก์ ต้องถูกสร้างแล้ว canonicalize
    #[test]
    fn test_new_creates_missing_allowed_root() {
        let dir = std::env::temp_dir().join(format!("nami_ws_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let missing = dir.join("docs");

        let guard = WorkspaceGuard::new(vec![missing.clone()], false).unwrap();
        assert!(missing.exists(), "missing allowed root must be auto-created");
        assert!(
            guard.allowed_roots[0].is_absolute(),
            "created root must canonicalize to absolute"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // validate_path บนไฟล์ที่มีอยู่จริงภายใต้ root แบบสัมพัทธ์ ต้องผ่าน
    #[test]
    fn test_validate_existing_file_within_relative_root() {
        let guard = WorkspaceGuard::new(vec![PathBuf::from("./")], false).unwrap();
        let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let validated = guard.validate_path(&target, false).unwrap();
        assert!(validated.is_absolute());
        assert!(validated.ends_with("Cargo.toml"));
    }

    // ไฟล์นอก root ต้องถูกปฏิเสธเสมอ (ทั้งแบบสัมพัทธ์ ../ และ absolute นอก workspace)
    #[test]
    fn test_validate_path_outside_root_rejected() {
        let guard = WorkspaceGuard::new(vec![PathBuf::from("./")], false).unwrap();
        let outside = std::env::temp_dir().join("nami_outside_scope_file.txt");
        assert!(guard.validate_path(&outside, false).is_err());
        assert!(guard.validate_path("../outside.txt", false).is_err());
    }

    // โหมดอ่านอย่างเดียว: ต้องปฏิเสธการเขียน แต่ยังอ่านได้
    #[test]
    fn test_read_only_mode_blocks_write() {
        let guard = WorkspaceGuard::new(vec![PathBuf::from("./")], true).unwrap();
        let err = guard.validate_path("new_file.txt", true).unwrap_err();
        assert!(err.to_string().contains("Read-Only mode"));
    }
}
