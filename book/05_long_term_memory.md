# Chapter 5: Long-term Memory – One SQLite File and a Folder of Markdown

Session history tells you what happened twenty minutes ago. That is not memory. Memory is
knowing what this project is, what we decided last month, and why the API is shaped the way
it is. For me, memory is two halves that deliberately live in different formats: a database
that is fast, and a folder of Markdown that is honest.

## 1. One Database File, Two Services

Both halves sit in the same SQLite file, pointed at by `storage.db_path`:

```rust
// src/memory.rs
let session_service = Arc::new(SqliteSessionService::new(&database_url).await?);
session_service.migrate().await?;

let memory_service = Arc::new(SqliteMemoryService::new(&database_url).await?);
memory_service.migrate().await?;
```

One file means one backup, one `git add`, one thing to delete when you want to reset me.
It also means sessions and memory share a lifespan — there is no way to lose one and keep
the other by accident.

Before either service opens, `ensure_sqlite_file` creates an empty file and any missing
parent directories. That is not decoration: the services open the path read/write and will
not create it for you, so without that helper a fresh clone fails to boot.

## 2. The Wiki Is the Source of Truth

The rule I follow is simple: **the Markdown files are real, the database is a cache.**

`./wiki/` is plain text. You can read it, edit it, review it in a pull request, and blame a
sentence to the commit that introduced it. No binary blob owns my knowledge. If the index
is wrong, I re-index; if the file is wrong, you fix the file.

## 3. Startup Re-Indexing: Wipe, Then Read

Every boot, I throw the old index away and rebuild it:

```rust
// src/memory.rs — NamiMemory::init
memory_service.delete_user("nami_app", "global_wiki").await?;

if wiki_dir.exists() {
    for entry in WalkDir::new(wiki_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.path().extension().and_then(|s| s.to_str()) == Some("md") {
            let content = std::fs::read_to_string(entry.path())?;
            let doc_name = entry.file_name().to_string_lossy();
            let memory_entry = MemoryEntry {
                content: Content::new("user")
                    .with_text(format!("Wiki [{}]: {}", doc_name, content)),
                author: "wiki_indexer".to_string(),
                timestamp: chrono::Utc::now(),
            };
            memory_service.add_entry("nami_app", "global_wiki", memory_entry).await?;
        }
    }
}
```

Three decisions live in that block, and each one was paid for with a bug:

- **Delete before add.** Indexing on top of the previous index accumulates duplicates on
  every restart. My first version did exactly that; after eight restarts a 90-line document
  was stored nine times.
- **Author is a fixed sentinel**, `"wiki_indexer"`. That makes the whole index deletable in
  one predicate later, without depending on filenames.
- **The content is prefixed** with `Wiki [<file>]: `. Retrieval hands the model raw text;
  without the prefix it has no idea whether it is looking at project knowledge or a user
  message.

Subdirectories are walked too, so files in subfolders index the same as top-level files in `wiki/`. Non-Markdown files are skipped silently.

## 4. The Self-Update Loop

Indexing is read-only, which means my memory only improves if something writes to it. The
write path is not a special memory API — it is the ordinary `write_file` tool from Chapter
10, pointed at `wiki/`.

That is the loop:

1. You tell me something that matters — a convention, a constraint, a decision.
2. I write it to the appropriate file under `wiki/`.
3. Next boot, it is indexed and I remember it forever.

No promotion pipeline, no trust tiers, no schema validation. It is deliberately the
simplest thing that can work, and the file is the artifact you keep if you throw me away.

## 5. What This Design Buys You

- **Git-friendly recall.** `git log wiki/` is a changelog of what I learned and when.
- **Zero lock-in.** My knowledge is not locked inside a vector store. Worst case, you
  `grep` it.
- **Cheap to inspect.** When I answer from stale knowledge, the fix is a text edit.

## 6. What It Does Not Do Yet

I will not pretend this is a retrieval stack:

- **No chunking.** Each document is stored whole. A very large file becomes a very large
  memory entry, and retrieval quality degrades as documents grow.
- **No embeddings.** Retrieval is whatever `adk-memory`'s SQLite backend does natively —
  there is no vector index and no semantic reranking layer in my build.
- **No incremental indexing.** Every boot is a full wipe-and-rebuild. Fine for a wiki the
  size of this one; not fine for ten thousand pages.

The Open Knowledge Format — typed concepts, provenance, attestation, trust tiers — is
designed but not implemented here. This chapter describes what runs today, and today that
is files plus a table.

Next: what happens when I get something wrong and have to fix myself mid-turn.
