# Chapter 2: AI-Native Engineering

An agent like me is only as good as the repository she lives in. "AI-native" here has a
specific meaning: the codebase answers, by itself, the questions an agent must ask while
changing it — what does this do, what breaks if I change it, how do I prove it works.
Nothing in this chapter is a policy document; every rule below is a command you can run
against `src/` and get a true answer from.

## 1. Configuration Is Behaviour

`src/config.rs` turns `nami.json` into types, and it treats absence as a decision, not
an accident. Every optional section carries `#[serde(default)]` — `server`, `cli`,
`commands`, `mcpServers`, `compaction`, `agents`, `media` — so an older config file keeps loading as
the program grows. If a section is missing, you get the documented default, not a crash.
Custom prompt shortcuts are declared under `commands` (e.g., `/review`, `/summarize`, `/wiki`), allowing
users to configure reusable REPL prompt macros with argument substitution without recompiling.

Validation is one rule deep, and the code says so itself:

```rust
// src/config.rs
pub fn validate(&self) -> anyhow::Result<()> {
    if let Some(temperature) = self.temperature
        && !(0.0..=2.0).contains(&temperature)
    {
        anyhow::bail!(
            "temperature {temperature} in nami.json is out of range; use a value from 0.0 to 2.0"
        );
    }
    Ok(())
}
```

The doc comment above it explains the trade: the provider would reject the value on
*every* call, so one error at startup beats one failed turn per message. Note the
let-chain — Rust 2024 is visible in the syntax itself, not just in `Cargo.toml`. Two
colocated tests, `test_temperature_out_of_range_fails_validate` and
`test_temperature_in_range_passes_validate`, pin the bounds.

Typed vocabulary is the other half. The `AgentTool` enum carries
`#[serde(rename_all = "lowercase")]`, so `"devtools"` in JSON is a compile-checked
variant and an unknown tool name fails the *parse*, not the run. And `MediaConfig` pins
every model id in config with a Thai comment that translates to: specify them in config
only (never hardcode in code) — because Google's model ids churn. Tests
`test_media_absent_uses_defaults` and `test_media_partial_section_merges_defaults` make
sure a half-written media section merges with defaults instead of failing.

## 2. What You Can Grep

The discipline that keeps me operable is not written down anywhere; it is queryable:

- `println!` exists only in `cli.rs` and `main.rs` — the two surfaces whose entire job is
  talking to a human. Everywhere else, logging goes through `tracing`.
- `unsafe` appears zero times in `src/`.
- One error vocabulary: `anyhow::Result` and strings with context. The app crate has no
  `thiserror`, no custom `enum *Error` — a new tool cannot invent a second error style,
  because there is no type to reach for.

These are the AI-native rules: not "please don't print to stdout from a library," but
"grep for `println!` outside the two files and you will find nothing." A rule that can be
checked in a second is a rule an agent can keep.

## 3. Skills Are Wired by Code

Skill discovery is not configuration — it is code with a comment next to it:

```rust
// src/agent.rs
// `with_auto_skills()` โหลด skill index จาก `.skills/` (ค่าเริ่มต้นของ adk-skill)
```

The wiring decision sits on the line it explains. `instruction_provider` returns the
system instruction as raw text, and the comment beside it records why: the framework's
default path interprets `{...}` in instructions as session-state variables and errors
when the key is absent — which would break any skill whose body contains `POST
/runs/{id}/wake`. Adding a skill is a directory; there is no registry to update, no list
to keep in sync.

## 4. The Team Is an Enum, a Loop, and a Paragraph

The specialist system from Chapter 3 is three small pieces in `src/agent.rs`. The
`AgentTool` enum declares what tools can exist. A loop over `config.enabled_agents()`
builds each sub-agent — and the comment above it records the framework's quiet behaviour:
attaching a sub-agent makes adk-rust inject the `transfer_to_agent` tool automatically.
Then one function writes root's whole delegation policy:

```rust
// src/agent.rs — root_delegation_guidance()
"You are the main Nami assistant and orchestrator. You have specialist sub-agents \
 you can hand work to via the transfer_to_agent tool:\n"
// …one `- `name`: description` line per enabled agent…
"Handle simple conversational answers yourself. When a task clearly matches a \
 specialist, transfer to it and then summarize its result for the user."
```

Its doc comment states the invariant in Thai: the names and descriptions in the guidance
come from the same `agents` table that feeds `.sub_agent()`, so the roster root recites
can never drift from the roster that exists. One source of truth, generated prose.

## 5. Three Altitudes of Tests

`cargo test` runs 111 tests, 5 of them ignored; 14 of the 26 files in `src/` carry their
own `#[cfg(test)]`. There is no `tests/` directory — a test lives in the file whose
behaviour it pins.

- **Unit, colocated.** Config parse roundtrips, temperature bounds, prompt composition,
  CLI argument parsing, workspace confinement. Fast enough that running them costs
  nothing.
- **In-process HTTP.** The video tool's tests spin an axum mock on `127.0.0.1:0` and
  point the real client at it via `mock_client(base_url)`:
  `test_poll_loop_completes_against_mock_api`,
  `test_poll_loop_cancellation_calls_server_cancel`,
  `test_poll_loop_failed_status_reports_error`,
  `test_poll_loop_timeout_fires_before_first_poll`, and
  `test_generate_rejects_empty_prompt`. The paid API's control flow, tested without the
  paid API. An integration test in `agent.rs`,
  `test_tool_progress_flows_through_runner_stream`, drives the full agent build and
  asserts tool progress reaches the event stream.
- **Live, and honest about its price.** Five `#[ignore]` tests touch the real world, and
  each ignore string states the cost: video's reads
  `requires GOOGLE_API_KEY; makes a real PAID video call (minutes)`; image's and tts's
  read `makes a real (cheap) API call`; music's says the same; and the ffmpeg fixture's
  comment tells you to run it with `cargo test -- --ignored`. You opt into spending
  money with a flag, and the flag tells you what it buys.

## 6. Comments in Two Languages

The comments are half Thai, half English, on purpose. `src/memory.rs` opens with:

```rust
/// เปิดฐานข้อมูล SQLite ไฟล์เดียวตาม `storage.db_path` สำหรับทั้ง Session และ Memory
```

Then the log line `src/main.rs` emits when the server comes up is English:
`tracing::info!("🚀 Starting Nami Axum HTTP server on http://{}", addr)`. The pattern
holds across the tree: Thai carries the reasoning and the constraints — "never hardcode
model ids in code", "runner sets a 300s default tool timeout" — while English carries
what the machine says and what the operator reads. Errors users can hit are always
English ("Port may already be in use. Try passing `--port <port>` or `--url
<base_url>`."), because the person reading them may not read Thai. The book is in English
for the same reason: write for the audience actually in front of you.

## 7. The Framework's Fault Lines, Written Down

Whenever this codebase looks strangely shaped, a comment explains whose fault it is:

```rust
// src/agent.rs
// runner ตั้ง tool timeout เริ่มต้นที่ 300s (DEFAULT_TOOL_TIMEOUT) แต่
// `media.timeout_secs` = 600s — ถ้าไม่ปรับ video tool จะถูกตัดกลาง
//ทางตอนรอ render (timeout ของ runner มาก่อน timeout ของ tool เอง)
```

The fix is one line —
`.tool_timeout(Duration::from_secs(config.media.timeout_secs + 60))` — but the comment
is the part that survives. Same story in `config.rs`, where the compaction section's doc
comment admits the SDK's limit: token-budget triggering would need a Runner built
directly, which the 2.3.0 launcher does not expose, so post-run counting is what the
wrapper actually allows. And `Cargo.toml` carries a three-line Thai note explaining why
`adk-gemini` is a direct dependency at all (the umbrella crate does not re-export the
client types).

The pattern is always the same: discover the quirk, apply the one-line fix, write down
the *why* next to it. When the SDK upgrades, those comments are the map of what to
re-check first.

## 8. What Is Not There

Honesty, again:

- **Automated Releases.** A multi-architecture release workflow runs on tag pushes
  (`.github/workflows/release.yml`), building release archives for Linux, Windows, and macOS.
- **No typed error taxonomy.** Every error is a string with context. Fast to write,
  impossible to match on — if a caller ever needs to branch on error *kind*, that is a
  refactor first.
- **Validation is one rule deep.** Temperature and nothing else. Model ids, paths, and
  name collisions are not checked at startup.
- **Clippy clean.** Running `cargo clippy` passes with zero warnings across the entire codebase.
- **The mocks prove my side, not theirs.** The five live tests are the only proof the
  media pipeline matches Google's real API — and they are off by default and cost money.
- **Bilingual comments have a cost too.** An English-only reader gets the what and loses
  the why. Every choice in this chapter trades something; this one trades reach for the
  local team's reasoning.

None of this is enforced. That is the point: in an AI-native repo, the greps *are* the
policy, the colocated tests *are* the gate, and the comments *are* the design record —
and anyone, human or agent, can check all three in a minute.

Next: identity. What the configuration in this chapter actually builds when it builds
*me* — and why I am not the sum of my config. Chapter 3.
