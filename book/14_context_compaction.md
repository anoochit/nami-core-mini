# Chapter 14: Context Compaction

Every message you send is appended to everything you have ever sent. That is what makes me
conversational and what eventually makes me expensive. Context compaction is the compromise:
summarise the old half, keep the recent half, and continue as if nothing was lost.

## 1. The Mechanism

Compaction is a post-run summariser configured on the server:

```rust
// src/compaction.rs
pub fn build_compaction_config(config: &NamiConfig) -> anyhow::Result<Option<EventsCompactionConfig>> {
    if !config.compaction.enabled {
        return Ok(None);
    }
    let model = build_model(config)?;
    Ok(Some(EventsCompactionConfig {
        compaction_interval: config.compaction.compaction_interval,
        overlap_size: config.compaction.overlap_size,
        summarizer: Arc::new(NamiEventsSummarizer::new(model)),
    }))
}
```

And it is attached in exactly one place, which is why every surface gets it:

```rust
// src/server.rs
if let Some(compaction) = crate::compaction::build_compaction_config(config)? {
    tracing::info!(interval = compaction.compaction_interval,
        "Enabled Context Compaction (post-run, every {} turns)", compaction.compaction_interval);
    server_config = server_config.with_compaction(compaction);
}
```

`chat`, `serve`, and `ask` all build their `ServerConfig` through that helper. Compaction
cannot be enabled for one mode and forgotten in another, because there is no second path.

## 2. The Config

```json
"compaction": {
  "enabled": true,
  "compaction_interval": 4,
  "overlap_size": 2,
  "context_window": 1048576
}
```

| Key | Meaning |
|-----|---------|
| `enabled` | `false` attaches no compaction at all |
| `compaction_interval` | Completed invocations between summaries — here, every 4th turn |
| `overlap_size` | Events carried from the previous window into the next summary, for continuity |
| `context_window` | Documented intent only — see section 5 |

Defaults if you omit the block: enabled, interval 4, overlap 2.

## 3. The Summariser

The summariser is my own model, doing the work:

```rust
// src/compaction.rs
async fn summarize_events(&self, events: &[Event]) -> Result<Option<Event>> {
    let prompt = build_summary_prompt(events);
    ...
    let mut event = Event::new("compaction");
    event.author = "system".to_string();
    event.actions = EventActions {
        compaction: Some(EventCompaction {
            start_timestamp,
            end_timestamp,
            compacted_content: Content::new("model")
                .with_text(format!("[Context Summary]\n{summary}")),
        }),
        ..Default::default()
    };
    Ok(Some(event))
}
```

The prompt is short and pointed:

> Summarize the following conversation history into a concise summary that preserves key
> facts, decisions, and context. Be brief but complete.

The resulting event records the timestamp range it covers, so the history knows *what period*
was replaced, not just that something was. And the `[Context Summary]` prefix marks the
replacement as a summary rather than as something you said — a small thing that stops me
later treating my own compression as a direct instruction from you.

Using `build_model` here means the summariser follows whatever provider `nami.json` names.
No separate summarization endpoint, no second API key.

## 4. What Compaction Costs

It is a real model call. Every fourth turn in the default configuration, I spend a request
summarising instead of answering, and the summary is lossy — specific phrasings, exact
numbers, and dead ends do not survive. `overlap_size: 2` exists to soften exactly this: two
events from the old window ride along into the next summary so the seam has something to
attach to.

If your work is dense with precise values — long refactors, financial figures, config keys —
raise the interval. Compaction optimises for "we have been talking a long time"; it is the
wrong tool for "we are twelve turns into a careful task".

## 5. The Honest Limitation

Compaction here is triggered by **counting invocations**, not by measuring the context window.

That is not a preference, it is a constraint. Token-budget triggering — "compact when the
history reaches 70% of the window" — requires constructing the `Runner` directly, and the
`Launcher`/`ServerConfig` path in adk-rust 2.3.0 does not expose that knob. The wrapper I run
through goes through `ServerConfig`, so the only trigger available is the turn count.

`context_window` is therefore carried in the config and **used for nothing yet**. It is
documentation of intent: when the runner path opens up, the field is already where it needs
to be. Do not expect changing it to alter behaviour today.

This also means compaction applies to the **serve** path only in the sense that all three
commands share it — there is no in-process CLI runner with hand-attached compaction any more.

## 6. Observing It

Watch the logs. The summariser emits one line per compaction:

```
INFO context compaction summarized history events_summarized=6
```

If a long session goes oddly forgetful, that line tells you when, and `nami.db` holds the
compaction event itself if you want to read what the summary said.

Next: how I decide to hand work to someone else.
