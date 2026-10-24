# Chapter 17: Automation Loops — Grill, Plan, Goal

Long tasks make models rot.

If you keep talking to me in a single 40-turn chat, my context window fills up, early instructions get
diluted, and subtle errors compound. Even worse, complex conversational chains with tool calls can cause
provider API errors when turn sequences fall out of strict user-model alternation.

The fix is not a bigger context window. The fix is a loop with fresh context and disk-backed memory.

In `nami chat`, this is implemented as a three-stage engineering pipeline:

1. `/grill`: Inquire, challenge assumptions, and resolve trade-offs (`src/grill.rs`).
2. `/plan`: Inspect the codebase and decompose the task into atomic steps (`src/plan.rs`).
3. `/goal`: Run an autonomous execution loop with external verification backpressure (`src/goal.rs`).

```
┌───────────────────────────────┐
│ /grill "<fuzzy idea>"         │ ──▶ Interrogates decisions ──▶ Saves GRILL.md
└───────────────────────────────┘
                │
                ▼
┌───────────────────────────────┐
│ /plan "<task or spec>"        │ ──▶ Explores codebase & breaks steps ──▶ Saves PLAN.md
└───────────────────────────────┘
                │
                ▼
┌───────────────────────────────┐
│ /goal "<step>" --until <check>│ ──▶ Ralph Wiggum fresh-context loop ──▶ Verified done!
└───────────────────────────────┘
```

---

## 1. `/grill`: The Architectural Inquisitor

The most expensive defect in software engineering is building the wrong thing brilliantly.

When given a high-level task like `"add user caching"`, the standard agent mistake is to guess the
storage layer, guess the invalidation strategy, and write 500 lines of code you never wanted.

`/grill` (`src/grill.rs`) flips the dynamic: I interview you before anyone writes code.

```rust
// src/grill.rs
pub fn build_grill_start_prompt(topic: &str) -> String {
    format!(
        r#"# Grilling Interview: {topic}

You are an expert Principal Software Architect conducting an intensive "Grilling Interview"...
## Rules of Engagement:
1. Inspect Codebase First: Use tools (`list_dir`, `read_file`, `search`) to inspect existing
   models and conventions. If a question can be answered from code, do NOT ask the user.
2. One Critical Decision at a Time: Focus on the most load-bearing decision first.
3. Always Recommend: Provide a concrete recommendation based on current codebase architecture.
4. Structured Format:
   ### Decision 1: [Topic/Branch]
   - Context: Why this decision is load-bearing.
   - Recommendation: [Your recommended choice with rationale].
   - Options: (a), (b), (c)
"#
    )
}
```

Two rules make `/grill` reliable:

1. **Codebase first:** I read `Cargo.toml` and existing modules before speaking. If `src/memory.rs`
   already uses SQLite, I don't ask you what database we use; I recommend using the existing SQLite
   database.
2. **Every question carries a recommendation:** You don't have to write an essay. You can simply
   type `"a"` or `"agree with recommended"`.

When the interview converges, typing `/grill done` triggers `build_grill_summary_prompt` and writes
a structured **`GRILL.md`** file recording all agreed technical locks, scope boundaries, and non-goals.

---

## 2. `/plan`: The Implementation Planner

Once requirements are sharp, `/plan "<task>"` (`src/plan.rs`) steps in as a Senior Software Architect.

```rust
// src/plan.rs
pub fn build_plan_prompt(task: &str, grill_context: Option<&str>) -> String {
    // Incorporates GRILL.md if present on disk
    // Instructs the model to inspect existing source files
    // Decomposes into atomic checklist steps
}
```

Instead of immediately generating code, `/plan`:

1. Checks for an existing `GRILL.md` and incorporates your agreed constraints.
2. Uses `read_file` and `list_dir` to map existing architecture.
3. Decomposes the work into atomic, testable phases.
4. Formats each step with an explicit `/goal` command:

   ```markdown
   - [ ] **Step 1: In-Memory Cache Store**
     - Details: Implement thread-safe LRU cache in `src/cache.rs`.
     - Files: `src/cache.rs`, `src/lib.rs`
     - Command: `/goal "implement lru cache in src/cache.rs" --until "cargo test cache"`
   ```

5. Automatically saves the output to **`PLAN.md`** (or `--out <path>`).

---

## 3. `/goal`: The Ralph Wiggum Autonomous Loop

The core execution engine lives in `src/goal.rs`. It implements the **Ralph Wiggum technique**
pioneered by Geoffrey Huntley:

> *"Ralph is a bash loop: `while :; do cat PROMPT.md | agent; done`"*

### The Fresh-Context Principle

Standard agent workflows keep appending turns to a single session. After 10 turns, the context is
stale and token-heavy.

In Nami's Ralph loop, **every single iteration creates a brand new session**:

```rust
// src/goal.rs
for iteration in 1..=spec.max_iterations {
    // 1. Always create a brand new session for each iteration to guarantee fresh context!
    client.new_session().await?;

    // 2. Build prompt with fresh context and failure feedback from previous iteration
    let prompt = build_ralph_prompt(
        &spec,
        iteration,
        spec.max_iterations,
        previous_feedback.as_deref(),
    );

    // 3. Stream agent execution
    let response = client.run_stream(&prompt).await?;
    let (stats, answer) = crate::cli::render_stream_response(response, true).await;
    ...
}
```

Because every iteration calls `client.new_session().await`, the context window is completely clean.
This guarantees:

- **Zero context degradation:** The model starts with 100% attention capacity on every iteration.
- **Zero turn-order bugs:** Prevents provider-level errors (such as Gemini's `400: Requests ending with a model turn are not supported`) because every loop begins with a clean user prompt.
- **State lives on disk:** My memory is not the chat history; it is the filesystem, git status, and test output.

---

## 4. Dual-Verification Backpressure

An autonomous loop without backpressure is a hallucination machine. I cannot simply ask myself if I am
done; I need objective verification.

`src/goal.rs` provides two verification paths:

```rust
// src/goal.rs
pub enum UntilMode {
    /// Deterministic shell command: exit code 0 means success.
    Command(String),
    /// Natural language acceptance criteria or expected result text.
    NaturalLanguage(String),
    /// No --until specified; relies on agent completion promise `<promise>DONE</promise>`.
    None,
}
```

### Path A: Deterministic Shell Verification

When you provide `--until "cargo test auth"` or `--until "npm test"`:

- The runner executes the command in an OS subprocess (`cmd.exe /C` on Windows, `sh -c` on Unix) with a timeout (`run_command_check`).
- If the exit code is `0`, the goal succeeds immediately.
- If non-zero, the failure tail (stdout/stderr) is captured and injected into the prompt of iteration $i+1$ as targeted feedback.

### Path B: Natural Language Acceptance Criteria

If you don't know the exact shell command and write:

```text
/goal "update README.md" --until "contains installation instructions and license"
```

The loop runs an **independent Evaluator turn** (`build_evaluator_prompt`):

1. A fresh session is spawned with an auditor persona.
2. The auditor inspects workspace files with filesystem tools.
3. It responds strictly with `<verified>YES</verified>` or `<verified>NO</verified>` plus a bulleted list of missing items.
4. If `<verified>NO</verified>`, the missing items are fed directly into the next iteration for correction.

---

## 5. Ctrl-C Safety

Long-running autonomous loops must be cancelable at any moment.

Both `run_ralph_loop` and `run_plan` wrap execution in `tokio::select!`:

```rust
// src/goal.rs
tokio::select! {
    res = run_ralph_loop_inner(client, spec) => res,
    _ = tokio::signal::ctrl_c() => {
        println!("\n\n[ralph] Interrupted (Ctrl-C). Stopping loop. Workspace changes preserved.\n");
        let _ = client.new_session().await;
        Ok(())
    }
}
```

Pressing `Ctrl-C` halts execution instantly, preserves all modified files in your workspace, and
restores a fresh session so you return cleanly to the `>` prompt in chat.

---

## 6. Real Turn in Chat

```text
> /grill "add api rate limiting"
  [grill] Inspecting workspace...
  Decision 1: Storage for rate limiter keys (Recommended: in-memory TokenBucket)
  > a
  [grill] All decisions resolved.
> /grill done
  📄 [grill] Decisions saved to: GRILL.md

> /plan "implement rate limiting based on GRILL.md"
  [plan] Generating Implementation Plan...
  📄 [plan] Implementation plan saved to: PLAN.md

> /goal "implement rate limit middleware in src/server.rs" --until "cargo test rate_limit"
  [ralph] Iteration 1/10 (Fresh Context)
  thinking: Creating test cases in tests/rate_limit.rs...
  tool: write_file: tests/rate_limit.rs
  [check 1/10] cargo test rate_limit -> exit 101 (failed)

  [ralph] Iteration 2/10 (Fresh Context with failure feedback)
  thinking: Fixing token replenishment timing...
  tool: edit_file: src/server.rs
  [check 2/10] cargo test rate_limit -> exit 0 (passed!)
  🎉 [ralph] Goal completed successfully in 2 iteration(s)!
```

Separation of concerns across the three commands makes autonomous coding predictable:

- `/grill` ensures we build the right thing.
- `/plan` ensures we build it in the right order.
- `/goal` ensures we don't stop until it actually works.

---

## 7. Serve Mode Background Runs & Cron Scheduling

Beyond interactive REPL loops, Nami in `serve` mode supports asynchronous task execution and recurring schedules powered by `adk-server`'s background feature:

- **Durable Background Runs (`/api/runs`):** Requests submitted to `/api/runs` execute asynchronously in background tasks managed by `BackgroundState` and `Runner`. Clients can inspect task status or stream results without keeping a long-lived synchronous connection open.
- **Cron Job Scheduler (`/api/cron`):** `CronState` and `start_cron_scheduler` enable periodic agent turns scheduled via standard cron expressions mounted under `/api/cron`.

This provides Nami with production-grade task orchestration and automated recurring execution directly inside the single Axum process.
