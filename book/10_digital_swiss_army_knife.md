# Chapter 10: The Digital Swiss Army Knife

Six tools, four files, no cleverness. This chapter documents what each one returns, because
the shape of a tool's output is what determines whether the model can recover when the tool
does not do what it hoped.

## 1. The Contract: Errors Are Data

First, the rule that governs all of my tools: **a failure is a successful call that returns
an error field.** Not a Rust `Err`, not a panic.

```rust
// src/tools/filesystem.rs
let target_path = match g.validate_path(path_str, false) {
    Ok(p) => p,
    Err(e) => return Ok(json!({ "error": e.to_string() })),
};
```

Every tool follows this. The reasoning is that a model reading `{"error": "File access
denied: path ... is outside the allowed scope"}` can retry with a different path; a model
presented with a transport-level failure often cannot tell what went wrong. Errors are for
the model to read, so they get returned in-band.

## 2. `read_file`, `write_file`, `list_files`

My filesystem trio, all wrapped in `WorkspaceGuard`:

| Tool | Arguments | Returns |
|------|-----------|---------|
| `read_file` | `path`, optional `offset`, `limit` | Line-numbered content, `total_lines` |
| `write_file` | `path`, `content` | `success`, `path`, `bytes_written` |
| `list_files` | optional `path`, `pattern` | `matches`, `count` |

Details that matter in practice:

- `read_file` numbers lines with six-column alignment (`{:6}\t`), so a 400-line file and a
  9-line file are both readable, and the model can reference a line number back to you.
- `write_file` creates parent directories automatically. It does not merge — it overwrites.
- `list_files` takes a glob, `**/*` by default, and backslashes are normalised to `/` so
  patterns behave the same on Windows.

```rust
// src/tools/filesystem.rs
let full_pattern = if target_dir.is_dir() {
    format!("{}/{}", target_dir.to_string_lossy().replace('\\', "/"), pattern)
} else {
    pattern.to_string()
};
```

Every one of these calls `validate_path` first, in both directions — read and write. That is
the guardrail from Chapter 18 doing its work at the only place it can.

## 3. `get_datetime`

Returns UTC and local time in RFC 3339, plus a Unix timestamp:

```json
{
  "datetime_utc": "2026-10-23T09:14:05.123+00:00",
  "datetime_local": "2026-10-23T16:14:05.123+07:00",
  "timezone_selected": "local",
  "timestamp": 1792746845
}
```

Honest note: the `timezone` argument is accepted and echoed back as `timezone_selected`, but
the tool always emits UTC and local regardless. It does not convert. Asking for `Asia/Tokyo`
gets you Bangkok and UTC with the word `Asia/Tokyo` printed underneath.

## 4. `get_weather_forecast`

One call to `wttr.in`, one line back:

```rust
// src/tools/weather.rs
let url = format!("https://wttr.in/{}?format=4", location);
```

`?format=4` is the single-line format, which is the whole point — a forecast is a fact you
want in one sentence of context, not a wall of ANSI art. No API key required, which makes
this the one tool here that works on a fresh clone with an empty `.env`.

## 5. `google_search`

The only tool that needs a key. `SERPER_API_KEY` gates it:

```rust
// src/tools/search.rs
let api_key = match std::env::var("SERPER_API_KEY") {
    Ok(key) if !key.is_empty() => key,
    _ => return Ok(json!({ "error": "SERPER_API_KEY not found in environment. ..." })),
};
```

It posts to `https://google.serper.dev/search`, clamps `num` to 1–10 (default 5), and then
does the interesting part — it throws most of the response away:

```rust
Ok(json!({
    "query": query,
    "answer_box": data.get("answerBox").cloned(),
    "knowledge_graph": data.get("knowledgeGraph").cloned(),
    "organic": data.get("organic").cloned().unwrap_or(Value::Array(vec![])),
}))
```

Serper's payload is enormous. Passing the raw JSON to the model would spend thousands of
tokens on metadata nobody reads. Keeping `answerBox`, `knowledgeGraph`, and `organic` keeps
the signal and drops the packaging.

## 6. Environment Variables

| Variable | Enables | Missing behaviour |
|----------|---------|-------------------|
| `SERPER_API_KEY` | `google_search` | Tool returns an error object; everything else works |
| `GOOGLE_API_KEY` | Gemini (default provider) | Warning printed at startup |
| `OPENAI_API_KEY` | `provider: "openai"` | Warning printed at startup |
| `ANTHROPIC_API_KEY` | `provider: "anthropic"` | Warning printed at startup |
| `OPENROUTER_API_KEY` | `provider: "openrouter"` | Warning printed at startup |

All are loaded by `dotenvy::dotenv()` at the top of `main`, so a `.env` file works without
exporting anything into your shell. A missing key never stops me from booting — it produces
a warning, or an error object from the tool that needs it. I would rather be partially
useful than refuse to start.

## 7. Adding a Seventh Tool

The pattern is fixed and small:

1. Define a params struct with `#[derive(JsonSchema, Serialize, Deserialize)]`.
2. Return `Ok(json!({...}))` for success *and* for failure.
3. Construct it with `FunctionTool::new(name, description, closure)`
   and `.with_parameters_schema::<YourParams>()`.
4. Attach it in `build_nami_agent` — to the root, and to any sub-agent whose `AgentTool`
   match arm should include it (which means extending the enum in `src/config.rs` too).

Next: how I get hands I was not born with.
