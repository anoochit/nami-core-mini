---
name: code-reviewer
description: Skill for reviewing Rust code against safety standards and idiomatic Rust practices. Use when asked to review, audit, or improve Rust code.
tags: [rust, code-review, security]
---

# Code Reviewer Skill Guidelines

When asked to review Rust code, follow these principles:

1. Error handling: use `Result` with `anyhow` / `thiserror` appropriately
2. Memory safety: avoid `unsafe` blocks unless the reason is necessary and clearly documented
3. Concurrency: use `tokio` async primitives and `Arc<RwLock<T>>` for shared state
4. Coding style: follow Rust Edition 2024 guidelines and the Clippy zero-warning policy
