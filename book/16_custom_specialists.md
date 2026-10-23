# Chapter 16: Custom Specialist Configuration

Chapter 15 explained how delegation works. This chapter is the operator's manual: how to
reshape my team without opening a Rust file. Everything here is the `agents` key in
`nami.json`.

## 1. The Schema

```rust
// src/config.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentConfig {
    /// Name used to target this agent via `transfer_to_agent`
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// System instruction for this sub-agent
    pub instruction: String,
    #[serde(default)]
    pub tools: Vec<AgentTool>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub disabled: bool,
}
```

| Field | Required | Effect |
|-------|----------|--------|
| `name` | Yes | The `transfer_to_agent` target; must be unique |
| `description` | No | Shown in the transfer enum and root's guidance; falls back to `name` |
| `instruction` | Yes | The specialist's system instruction |
| `tools` | No | Allowlist of built-in tools; empty means **no tools at all** |
| `model` | No | Model override; inherits root's model when omitted |
| `disabled` | No | Skipped entirely at build time |

Omit the whole `agents` key and nothing breaks — root works alone with no transfer tool.

## 2. The Tool Vocabulary

`tools` accepts exactly six values, lowercased:

```rust
// src/config.rs
#[serde(rename_all = "lowercase")]
pub enum AgentTool {
    Search,     // "search"     — google_search via Serper
    Devtools,   // "devtools"   — workspace read/write/edit/glob/grep + bash
    Filesystem, // "filesystem" — read_file, write_file, list_files (WorkspaceGuard)
    Datetime,   // "datetime"   — get_datetime
    Weather,    // "weather"    — get_weather_forecast
    Media,      // "media"      — image/TTS/music/video/assemble (Chapter 11)
}
```

`media` is the one value with a second gate: it also requires `"media": { "enabled": true }`
in `nami.json`. Request it while media is disabled and the agent starts with no tools plus a
warning naming it.

Anything else is a parse error at startup, not a silent omission — which is the right
failure mode, since a typo would otherwise produce a specialist mysteriously missing a tool.

## 3. A Specialist Worth Adding

Here is one that is not in the shipped config but that the current architecture handles
cleanly:

```json
{
  "name": "librarian",
  "description": "Answers questions about this project's own documentation",
  "instruction": "You answer questions about this project using only its wiki/ documents. Cite the file name you drew from. If the wiki does not cover it, say so rather than guessing.",
  "tools": [],
  "model": "gemini-3.5-flash-lite"
}
```

Two things make it interesting. An **empty tool list** makes it a pure reasoning agent over
context — cheap, safe, and impossible to accidentally let loose on the filesystem. And a
**model override** sends it to a faster, cheaper model, since reading documents does not
need the reasoning model root uses.

The override path is real, not aspirational:

```rust
// src/agent.rs
let sub_model = match &spec.model {
    Some(override_model) => build_model_with(config, Some(override_model))?,
    None => model.clone(),
};
```

One caveat: `model` is a model *name*, not a provider. The provider stays whatever root
uses. I cannot route one specialist to Anthropic and another to Gemini — that would need a
provider field per agent, which does not exist yet.

## 4. Turning Specialists Off

`disabled: true` removes an agent from both the tool enum and the guidance text, because
both are generated from `enabled_agents()`:

```rust
// src/config.rs
pub fn enabled_agents(&self) -> impl Iterator<Item = &SubAgentConfig> {
    self.agents.iter().filter(|a| !a.disabled)
}
```

This is the feature I would call most underused. Set `"disabled": true` on three of your four
specialists and you have a cheaper, faster assistant for the afternoon — no rebuild, no
commented-out blocks, and the config still documents what the full team looks like.

## 5. Why the Guidance Cannot Go Stale

There is a specific bug this design prevents. Suppose the transfer tool's enum were built
from `agents` while root's guidance text were hand-maintained. Remove a specialist and the
enum drops it — but the paragraph still tells me to transfer to it. Now I am calling a tool
variant that does not exist.

Both come from the same iterator, so that cannot happen. The guidance is generated *from*
the registration site, at build time, in `root_delegation_guidance`. There is no second copy
of the roster to forget.

## 6. Common Configurations

| Goal | Change |
|------|--------|
| Cheaper daily driver | Disable everything except one specialist |
| Stronger reviews | Give `reviewer` its own `model` |
| Read-only research bot | `researcher` with `["search"]` only, `devtools` removed |
| Pure Q&A agent | Drop the `agents` key entirely |
| Restrict root too | `workspace.read_only: true` (Chapter 18) |

## 7. Editing Config Safely

There is no `nami validate`. An invalid `agents` array fails when `serde_json` parses
`nami.json` at startup, before anything is built — which is early and clear, but only when
you restart. The parser does not care about key order or trailing commas being absent
(Jones's law applies: JSON has one right way), but it does care about types: `"tools": "devtools"`
is an error, `"tools": ["devtools"]` is not.

Next: the guardrails, because a config key that grants filesystem access deserves its own
chapter.
