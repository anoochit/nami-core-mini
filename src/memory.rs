use adk_memory::{MemoryEntry, MemoryService, SqliteMemoryService};
use adk_rust::prelude::Content;
use adk_session::SqliteSessionService;
use anyhow::Context;
use std::path::Path;
use std::sync::Arc;
use walkdir::WalkDir;

#[allow(dead_code)]
pub struct NamiMemory {
    pub session_service: Arc<SqliteSessionService>,
    pub memory_service: Arc<SqliteMemoryService>,
}

impl NamiMemory {
    /// Open single SQLite database file according to `storage.db_path` for both Session and Memory,
    /// then index Markdown documents from `wiki/` directory into Long-term Memory
    pub async fn init(db_path: &Path, wiki_dir: &Path) -> anyhow::Result<Self> {
        ensure_sqlite_file(db_path)?;
        let database_url = format!("sqlite:{}", db_path.display());

        let session_service = Arc::new(SqliteSessionService::new(&database_url).await?);
        session_service.migrate().await?;

        let memory_service = Arc::new(SqliteMemoryService::new(&database_url).await?);
        memory_service.migrate().await?;

        // Clear existing wiki index before re-indexing on startup to prevent duplicate entries
        memory_service
            .delete_user("nami_app", "global_wiki")
            .await?;

        // Index Markdown files from wiki/ into Memory Store
        if wiki_dir.exists() {
            // WalkDir เป็น blocking traversal — ย้ายไปรันบน blocking thread pool
            // เพื่อไม่ให้ไปยึด Tokio runtime worker
            let wiki_dir = wiki_dir.to_path_buf();
            let md_files = tokio::task::spawn_blocking(move || {
                WalkDir::new(wiki_dir)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("md"))
                    .map(|e| e.path().to_path_buf())
                    .collect::<Vec<_>>()
            })
            .await
            .context("Wiki indexing traversal failed")?;

            for file_path in md_files {
                // ใช้ tokio::fs กัน blocking call ภายใน async task
                let content = tokio::fs::read_to_string(&file_path)
                    .await
                    .with_context(|| format!("Failed to read wiki document: {:?}", file_path))?;
                let doc_name = file_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                tracing::info!("Indexing Local Wiki Document: {}", doc_name);

                // Save wiki document into Long-term Memory
                let memory_entry = MemoryEntry {
                    content: Content::new("user")
                        .with_text(format!("Wiki [{}]: {}", doc_name, content)),
                    author: "wiki_indexer".to_string(),
                    timestamp: chrono::Utc::now(),
                };
                memory_service
                    .add_entry("nami_app", "global_wiki", memory_entry)
                    .await?;
            }
        }

        Ok(Self {
            session_service,
            memory_service,
        })
    }
}

/// Create empty database file (and parent directory) if it does not exist
///
/// `SqliteSessionService::new` opens file in read/write mode without auto-creating it,
/// so a 0-byte file must be prepared beforehand which SQLite treats as a valid empty DB.
fn ensure_sqlite_file(db_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    if !db_path.exists() {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(db_path)?;
    }
    Ok(())
}
