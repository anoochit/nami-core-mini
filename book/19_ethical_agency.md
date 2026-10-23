# Chapter 19: Ethical Agency – The Nami Trust Protocol

Security guardrails (Chapter 18) define what I *can* do mechanically — path confinement, read-only flags, loopback server bindings. Ethical agency governs how I behave when actions are technically allowed, but require judgment, restraint, and verification.

Giving an agent the ability to execute shell commands, edit codebases, and run autonomous loops (`/goal`) requires an explicit trust model. The **Nami Trust Protocol** is the set of engineering principles that keep my autonomous execution safe, accountable, and aligned with user intent.

## 1. The Verification Contract

The fundamental ethical hazard of autonomous AI agents is **hallucinated success** — an agent completing a task, encountering an error, and convincing itself that the work is finished when it is not.

To eliminate hallucinated success, my automation engine (`src/goal.rs`) enforces strict verification boundaries:

- **Deterministic Verification:** Whenever possible, goals use shell command verification (`--until "cargo test"`). The goal loop does not exit because the model says "all tests pass"; it exits only when the operating system returns exit code `0`.
- **Dual-Context Natural Language Evaluation:** When natural language criteria are used, evaluation happens in a fresh, isolated evaluation turn (`eval_turn`). The model evaluating the result is not the model that wrote the code — preventing self-confirmation bias.
- **Completion Promise Tagging:** The loop requires an explicit promise tag (`<promise>DONE</promise>`). Unstructured chatter or incomplete turns cannot accidentally trigger goal completion.

## 2. The Minimal Change Principle

When granted write permissions inside a workspace, an unchecked agent can easily refactor unrelated modules, delete legacy files, or introduce massive churn that makes code review impossible.

My sub-agent system enforces the **Minimal Change Principle** directly in system instructions:

```json
// nami.json — coder sub-agent
"instruction": "You are a coding specialist. Use the workspace tools to read, write, and edit files, and to run shell commands. Make minimal, correct changes and explain them."
```

By instructing specialists to make minimal changes, the blast radius of any individual turn is contained. Small, targeted diffs make agent execution predictable, easy to inspect, and simple to revert via version control (`git diff`).

## 3. Transparency & Auditability

Ethical agency requires total transparency. An agent whose reasoning is hidden behind opaque background queues or unlogged threads cannot be trusted.

In Nami Mini, every turn leaves a complete, queryable audit trail:

- **Streaming Telemetry:** Every tool call (`🔧 call`), tool result (`✅` / `❌`), thinking phase (`🤔 thinking`), and final metrics (`💭 thought`) are rendered live to stdout over SSE (`src/cli.rs`).
- **Persistent Session State:** All conversation turns, tool calls, and model outputs are recorded in SQLite (`nami.db`) via `adk-session` (Chapter 13).
- **Workspace Visibility:** Memory and wiki files (`wiki/`) are stored as plain text. You can inspect, edit, or `git blame` every fact I record.

Nothing occurs in unmonitored background tasks.

## 4. Self-Correction over Panic

When a tool call fails — due to a missing file, a syntax error, or a failed shell command — the ethically responsible behavior is not to panic or silently drop the task.

Through `adk-retry-reflect` (Chapter 6), failed tool invocations are intercepted and fed back to the model wrapped in a reflection prompt:

1. Tool execution returns an error.
2. The reflection plugin intercepts the failure.
3. Up to three retries are granted, forcing the model to reflect on the error message and modify its approach.

Self-correction turns tool errors into diagnostic feedback loops, ensuring that minor hiccups do not result in corrupted file states.

## 5. What Is Missing

To remain honest about the current build:

- **No Interactive Tool Approval:** I do not pause before executing dangerous shell commands or editing files to ask for user confirmation. If the tool is enabled in `nami.json`, I execute it immediately.
- **No Automatic Secret Scanning:** I do not scan files before writing them to ensure API keys or passwords are not accidentally written into public workspace files.
- **No PII Redaction:** Output text is not filtered for personally identifiable information before streaming to the client.

## 6. The Rules of Ethical Agency

When operating or configuring Nami in production environments:

1. **Verify deterministically:** Use shell verification commands (`cargo test`, `npm test`) for autonomous goal loops whenever possible.
2. **Review diffs before committing:** Use `git status` and `git diff` to inspect changes made during an execution session.
3. **Isolate sensitive credentials:** Ensure `.env` is gitignored and API keys are never written to project files.
4. **Scope tool access:** Grant only the toolsets required for a task (e.g., `researcher` gets `search`, `coder` gets `devtools`).

Next: looking ahead — from an interactive assistant to an autonomous software architect. Chapter 20.
