//! Context Compaction for Nami Mini
//!
//! Uses `EventsCompactionConfig` (post-run turn-count based compaction) from adk-rust to
//! periodically summarize older conversation history. Token-budget based compaction ("70% of context")
//! requires direct `Runner` construction, which `Launcher`/`ServerConfig` in adk-rust 2.3.0 does not expose —
//! hence compaction is currently configured per turn interval in server mode.

use adk_rust::async_trait;
use adk_rust::{
    BaseEventsSummarizer, Content, Event, EventActions, EventCompaction, EventsCompactionConfig,
    Llm, LlmRequest, Part, Result,
};
use futures::StreamExt;
use std::sync::Arc;

use crate::agent::{build_model, effective_temperature};
use crate::config::NamiConfig;

/// Summarizer using application LLM to compact event window into a single summary event
pub struct NamiEventsSummarizer {
    model: Arc<dyn Llm>,
    /// Application sampling temperature — summary uses same value as standard turns
    temperature: Option<f32>,
}

impl NamiEventsSummarizer {
    pub fn new(model: Arc<dyn Llm>, temperature: Option<f32>) -> Self {
        Self { model, temperature }
    }
}

#[async_trait]
impl BaseEventsSummarizer for NamiEventsSummarizer {
    async fn summarize_events(&self, events: &[Event]) -> Result<Option<Event>> {
        if events.is_empty() {
            return Ok(None);
        }

        let prompt = build_summary_prompt(events);
        let mut request = LlmRequest::new(
            self.model.name().to_string(),
            vec![Content::new("user").with_text(prompt)],
        );
        if let Some(temperature) = self.temperature {
            request.config = Some(adk_rust::GenerateContentConfig {
                temperature: Some(temperature),
                ..Default::default()
            });
        }

        let mut stream = self.model.generate_content(request, false).await?;
        let mut summary = String::new();
        while let Some(response) = stream.next().await {
            let response = response?;
            if let Some(content) = &response.content {
                for part in &content.parts {
                    if let Part::Text { text } = part {
                        summary.push_str(text);
                    }
                }
            }
        }

        let start_timestamp = events.first().expect("events is non-empty").timestamp;
        let end_timestamp = events.last().expect("events is non-empty").timestamp;

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

        tracing::info!(
            events_summarized = events.len(),
            "context compaction summarized history"
        );
        Ok(Some(event))
    }
}

/// Construct summary prompt from event list
fn build_summary_prompt(events: &[Event]) -> String {
    let mut prompt = String::from(
        "Summarize the following conversation history into a concise summary \
         that preserves key facts, decisions, and context. Be brief but complete.\n\n",
    );
    for event in events {
        if let Some(content) = event.content() {
            prompt.push_str(&format!("[{}]: ", content.role));
            for part in &content.parts {
                if let Part::Text { text } = part {
                    prompt.push_str(text);
                }
            }
            prompt.push('\n');
        }
    }
    prompt
}

/// Construct `EventsCompactionConfig` from `NamiConfig` when compaction is enabled
///
/// Returns `None` when `compaction.enabled` is `false`
pub fn build_compaction_config(
    config: &NamiConfig,
) -> anyhow::Result<Option<EventsCompactionConfig>> {
    if !config.compaction.enabled {
        return Ok(None);
    }
    let model = build_model(config)?;
    Ok(Some(EventsCompactionConfig {
        compaction_interval: config.compaction.compaction_interval,
        overlap_size: config.compaction.overlap_size,
        summarizer: Arc::new(NamiEventsSummarizer::new(
            model,
            effective_temperature(config, &config.model),
        )),
    }))
}
