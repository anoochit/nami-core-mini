//! REPL CLI mode for Nami Mini (HTTP client)

use adk_rust::{Event, EventTextDeltas, Part, UsageMetadata};
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};

use crate::api_client::{NamiApiClient, SseParser};

#[derive(PartialEq, Clone, Copy)]
enum Section {
    None,
    Thinking,
    Answer,
    Tool,
}

/// Print a section header on its own line, preceded by a blank line
/// (except before the very first output). Content follows on the next line.
fn print_section_header(label: &str, first_output: &mut bool) {
    if *first_output {
        print!("\n{label}:\n\n");
        *first_output = false;
    } else {
        print!("\n{label}:\n\n");
    }
    io::stdout().flush().ok();
}

/// Token totals for a turn (summed across all model calls in the turn)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenTotals {
    pub prompt: i64,
    pub output: i64,
    pub thinking: i64,
    pub total: i64,
}

/// Turn metrics displayed after answer in chat mode: model thinking time and token usage
#[derive(Debug, Default)]
pub struct TurnStats {
    /// Thinking duration for the turn — sum of thinking phases when measured,
    /// or time from stream start to first output as fallback when model emits no thinking
    thought: Duration,
    /// true when `thought` duration is available to display
    thinking_seen: bool,
    /// `None` when provider returns no usage metadata
    tokens: Option<TokenTotals>,
}

/// Tracks timing and token usage during a turn stream
struct TurnTracker {
    stats: TurnStats,
    /// Usage stored by event id: partial chunks for the same model call share an id,
    /// so the latest overwrites previous ones before final summation
    usage_by_event: HashMap<String, UsageMetadata>,
    /// Start instant of an open thinking phase (`None` if not currently thinking)
    thinking_started: Option<Instant>,
    /// First output of the turn (answer or tool call) used as thinking time fallback
    first_output_at: Option<Instant>,
}

impl TurnTracker {
    fn new() -> Self {
        Self {
            stats: TurnStats::default(),
            usage_by_event: HashMap::new(),
            thinking_started: None,
            first_output_at: None,
        }
    }

    /// Record event usage (skipped if provider sends no usage)
    fn on_event(&mut self, event: &Event) {
        if let Some(usage) = &event.llm_response.usage_metadata {
            self.usage_by_event.insert(event.id.clone(), usage.clone());
        }
    }

    /// Start timing thinking phase when first thinking delta arrives
    fn on_thinking(&mut self, now: Instant) {
        if self.thinking_started.is_none() {
            self.thinking_started = Some(now);
            self.stats.thinking_seen = true;
        }
    }

    /// Close thinking phase when output (answer or tool call) follows —
    /// tool execution time is excluded from thinking time
    fn on_output(&mut self, now: Instant) {
        if self.first_output_at.is_none() {
            self.first_output_at = Some(now);
        }
        self.close_thinking_phase(now);
    }

    /// Accumulate any active thinking phase into `thought`
    fn close_thinking_phase(&mut self, now: Instant) {
        if let Some(started) = self.thinking_started.take() {
            self.stats.thought += now.saturating_duration_since(started);
        }
    }

    /// Close remaining thinking phase and summarize token usage for all events
    ///
    /// `started` is the stream start instant — used as fallback thinking duration when
    /// thinking cannot be measured directly (e.g. model emits no thinking deltas)
    fn finish(&mut self, started: Instant, now: Instant) {
        self.close_thinking_phase(now);
        if self.stats.thought.is_zero()
            && let Some(first_output) = self.first_output_at
        {
            self.stats.thought = first_output.saturating_duration_since(started);
            self.stats.thinking_seen = true;
        }
        if self.usage_by_event.is_empty() {
            return;
        }

        let mut totals = TokenTotals::default();
        for usage in self.usage_by_event.values() {
            totals.prompt += i64::from(usage.prompt_token_count);
            totals.output += i64::from(usage.candidates_token_count);
            totals.thinking += i64::from(usage.thinking_token_count.unwrap_or(0));
            totals.total += i64::from(usage.total_token_count);
        }
        self.stats.tokens = Some(totals);
    }
}

/// Formats summary line "thought time + token count" — returns `None` when nothing to display
///
/// Example output: `💭 thought 1.8s · tokens 1,240 (prompt 900 · output 160 · thinking 180)`
pub fn stats_line(stats: &TurnStats) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if stats.thinking_seen {
        parts.push(format!("💭 thought {}", format_duration(stats.thought)));
    }

    if let Some(tokens) = stats.tokens {
        let mut breakdown = Vec::new();
        if tokens.prompt > 0 {
            breakdown.push(format!("prompt {}", fmt_int(tokens.prompt)));
        }
        if tokens.output > 0 {
            breakdown.push(format!("output {}", fmt_int(tokens.output)));
        }
        if tokens.thinking > 0 {
            breakdown.push(format!("thinking {}", fmt_int(tokens.thinking)));
        }

        let total = if tokens.total > 0 {
            tokens.total
        } else {
            tokens.prompt + tokens.output + tokens.thinking
        };
        if total > 0 {
            parts.push(if breakdown.is_empty() {
                format!("tokens {}", fmt_int(total))
            } else {
                format!("tokens {} ({})", fmt_int(total), breakdown.join(" · "))
            });
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

/// Duration formatting: milliseconds when < 1 second, else decimal seconds to 1 decimal place
fn format_duration(duration: Duration) -> String {
    if duration.as_millis() < 1000 {
        format!("{}ms", duration.as_millis())
    } else {
        format!("{:.1}s", duration.as_secs_f64())
    }
}

/// Format integer with thousands commas, e.g., `1240` → `1,240`
fn fmt_int(value: i64) -> String {
    let digits = value.abs().to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if value < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

/// Summarize tool response into lines for the `tool:` section
///
/// - `error` (Nami tool convention) → `❌` line
/// - `outputs` (media tools: path + bytes + duration) → `✅` line + list of paths
/// - other → `✅` line + short JSON (truncated at 160 chars)
fn tool_result_lines(name: &str, response: &Value) -> Vec<String> {
    if let Some(err) = response.get("error").and_then(Value::as_str) {
        return vec![format!("❌ {name}: {}", truncate_line(err, 200))];
    }

    if let Some(outputs) = response.get("outputs").and_then(Value::as_array) {
        let mut lines = vec![format!("✅ {name}")];
        for output in outputs {
            let path = output.get("path").and_then(Value::as_str).unwrap_or("?");
            let mut meta: Vec<String> = Vec::new();
            if let Some(bytes) = output.get("bytes").and_then(Value::as_u64) {
                meta.push(fmt_bytes(bytes));
            }
            if let Some(secs) = output.get("duration_seconds").and_then(Value::as_f64) {
                meta.push(format!("{secs:.1}s"));
            }
            if meta.is_empty() {
                lines.push(format!("  {path}"));
            } else {
                lines.push(format!("  {path} ({})", meta.join(" · ")));
            }
        }
        return lines;
    }

    let text = if let Some(s) = response.as_str() {
        s.to_string()
    } else {
        response.to_string()
    };
    vec![format!("✅ {name}: {}", truncate_line(&text, 160))]
}

/// Format progress chunk to single line: trim + newlines → spaces
///
/// Example: `"still rendering — 30s elapsed\n"` → `"… still rendering — 30s elapsed"`
fn format_progress_chunk(text: &str) -> String {
    let flat = text.replace(['\n', '\r'], " ");
    format!(
        "… {}",
        flat.split_whitespace().collect::<Vec<_>>().join(" ")
    )
}

/// Truncate long line: collapse whitespace and truncate at `max` characters (char boundary)
fn truncate_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let cut: String = flat.chars().take(max).collect();
        format!("{cut}…")
    }
}

/// Human-readable file size: B / KB / MB
fn fmt_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / KB)
    } else {
        format!("{:.1} MB", bytes as f64 / MB)
    }
}

use crate::config::DisplayOptions;

/// Render streamed SSE events from an HTTP response to stdout.
/// Configured via [`DisplayOptions`] to show/hide thinking, tool calls, and tool results.
///
/// Returns [`TurnStats`] for the turn (thinking duration + token count) for caller display
pub async fn render_stream_response(
    response: reqwest::Response,
    display: DisplayOptions,
) -> (TurnStats, String) {
    let mut bytes_stream = response.bytes_stream();
    let mut parser = SseParser::new();
    let mut text_deltas = EventTextDeltas::default();
    let mut tracker = TurnTracker::new();
    let started = Instant::now();
    let mut section = Section::None;
    let mut first_output = true;
    let mut accumulated_answer = String::new();

    while let Some(chunk_res) = bytes_stream.next().await {
        let chunk = match chunk_res {
            Ok(c) => c,
            Err(e) => {
                eprintln!("\n[error] Stream error: {}", e);
                tracker.finish(started, Instant::now());
                return (tracker.stats, accumulated_answer);
            }
        };

        let events = parser.push_bytes(&chunk);
        for evt in events {
            if let Some(msg) = &evt.llm_response.error_message
                && !msg.is_empty()
            {
                eprintln!("\n[error] {}", msg);
                tracker.finish(started, Instant::now());
                return (tracker.stats, accumulated_answer);
            }

            tracker.on_event(&evt);
            let now = Instant::now();

            let evt = text_deltas.push(&evt);

            // Tool progress chunk (e.g. video render) — rendered directly in tool section
            // without modifying stats (no usage / no output timing)
            if evt.tool_progress_stream().is_some() {
                if (display.show_tool_call || display.show_tool_result)
                    && let Some(content) = evt.content()
                {
                    for part in &content.parts {
                        if let Part::Text { text } = part
                            && !text.is_empty()
                        {
                            if section != Section::Tool {
                                print_section_header("🛠️ tool", &mut first_output);
                                section = Section::Tool;
                            }
                            println!("{}", format_progress_chunk(text));
                            io::stdout().flush().ok();
                        }
                    }
                }
                continue;
            }

            if let Some(content) = evt.content() {
                for part in &content.parts {
                    match part {
                        Part::Text { text } if !text.is_empty() => {
                            accumulated_answer.push_str(text);
                            tracker.on_output(now);
                            if display.show_answer && section != Section::Answer {
                                print_section_header("💬 answer", &mut first_output);
                                section = Section::Answer;
                            }
                            print!("{text}");
                            io::stdout().flush().ok();
                        }
                        Part::Thinking { thinking, .. } if !thinking.is_empty() => {
                            tracker.on_thinking(now);
                            if display.show_thinking {
                                if section != Section::Thinking {
                                    print_section_header("🤔 thinking", &mut first_output);
                                    section = Section::Thinking;
                                }
                                print!("{thinking}");
                                io::stdout().flush().ok();
                            }
                        }
                        Part::FunctionCall { name, args, .. } => {
                            // Partial event of model stream — final (non-partial) event will
                            // carry same call, so print only on committed event
                            if evt.llm_response.partial {
                                continue;
                            }
                            tracker.on_output(now);
                            if display.show_tool_call {
                                if section != Section::Tool {
                                    print_section_header("🛠️ tool", &mut first_output);
                                    section = Section::Tool;
                                }
                                println!("🔧 {name}({args})");
                                io::stdout().flush().ok();
                            }
                        }
                        // Tool result (media: path + metadata, other: short JSON)
                        Part::FunctionResponse {
                            function_response, ..
                        } => {
                            if evt.llm_response.partial {
                                continue;
                            }
                            tracker.on_output(now);
                            if display.show_tool_result {
                                if section != Section::Tool {
                                    print_section_header("🛠️ tool", &mut first_output);
                                    section = Section::Tool;
                                }
                                for line in tool_result_lines(
                                    &function_response.name,
                                    &function_response.response,
                                ) {
                                    println!("{line}");
                                }
                                io::stdout().flush().ok();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    tracker.finish(started, Instant::now());
    (tracker.stats, accumulated_answer)
}

/// Expands a custom slash command from `config.commands`.
///
/// Returns `Some(expanded_prompt)` if `cmd_name` matches a configured shortcut.
/// Supports optional `{args}` placeholder in template, or appends args on a new line.
pub fn expand_custom_command(
    commands: &HashMap<String, String>,
    line: &str,
) -> Option<String> {
    if !line.starts_with('/') {
        return None;
    }
    let rest = line[1..].trim();
    if rest.is_empty() {
        return None;
    }
    let mut parts = rest.splitn(2, char::is_whitespace);
    let cmd_name = parts.next()?;
    let args = parts.next().unwrap_or("").trim();

    let template = commands.get(cmd_name)?;

    if template.contains("{args}") {
        Some(template.replace("{args}", args))
    } else if !args.is_empty() {
        Some(format!("{template}\n\nContext: {args}"))
    } else {
        Some(template.clone())
    }
}

/// Checks if a slash command is a built-in REPL command.
pub fn is_builtin_command(line: &str) -> bool {
    let cmd = line.split_whitespace().next().unwrap_or("");
    matches!(
        cmd,
        "exit"
            | "quit"
            | "/exit"
            | "/quit"
            | "/new"
            | "/session"
            | "/help"
            | "/grill"
            | "/plan"
            | "/goal"
    )
}

/// Runs interactive REPL, reads lines from stdin and sends requests via NamiApiClient
pub async fn run_repl(client: &mut NamiApiClient, config: &crate::config::NamiConfig) -> anyhow::Result<()> {
    println!();
    println!("  Nami - ({})", client.app_name());
    println!("  Type a message to chat. Type 'exit' to quit or '/help' for options.");
    println!();

    let mut grill_session = crate::grill::GrillSessionState::default();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(32);
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut lines = stdin.lock().lines();
        while let Some(Ok(line)) = lines.next() {
            if tx.blocking_send(line).is_err() {
                break;
            }
        }
    });

    loop {
        print!("> ");
        io::stdout().flush().ok();

        let Some(line) = rx.recv().await else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if matches!(trimmed, "exit" | "quit" | "/exit" | "/quit") {
            println!("\nGoodbye.\n");
            break;
        }

        if trimmed == "/new" {
            grill_session.active = false;
            match client.new_session().await {
                Ok(_) => println!("Switched to new session: {}\n", client.session_id()),
                Err(e) => eprintln!("Failed to create new session: {}\n", e),
            }
            continue;
        }

        if trimmed == "/session" {
            println!("URL:        {}", client.base_url());
            println!("App Name:   {}", client.app_name());
            println!("Session ID: {}\n", client.session_id());
            println!("Provider:   {}", config.provider);
            println!("Model:      {}", config.model);
            continue;
        }

        if trimmed == "/help" {
            println!("Commands:");
            println!("  /new      - Start a new conversation session");
            println!("  /session  - Show current session info");
            println!("  /grill    - Interview to clarify requirements and write GRILL.md");
            println!("  /plan     - Generate structured implementation plan (PLAN.md)");
            println!("  /goal     - Run autonomous Ralph Wiggum development loop");
            println!("  /help     - Show this help message");
            println!("  exit      - Exit Nami");
            if !config.commands.is_empty() {
                println!();
                println!("Custom Shortcuts (nami.json):");
                for (name, template) in &config.commands {
                    println!("  /{:<8} - {}", name, truncate_line(template, 60));
                }
            }
            println!();
            continue;
        }

        if trimmed == "/grill" || trimmed.starts_with("/grill ") {
            let rest = trimmed.strip_prefix("/grill").unwrap().trim();
            match crate::grill::tokenize_grill_args(rest) {
                Ok(crate::grill::GrillCommand::Help) => {
                    println!("Grilling Interview Commands:");
                    println!(
                        "  /grill \"<topic>\" [--out <path>] - Start an architectural interview"
                    );
                    println!(
                        "  /grill done [--out <path>]       - Compile decisions into GRILL.md"
                    );
                    println!(
                        "  /grill status                    - Show active grilling session state"
                    );
                    println!(
                        "  /grill reset                     - Discard active grilling session"
                    );
                    println!("  /grill --help                    - Show this help message\n");
                }
                Ok(crate::grill::GrillCommand::Status) => {
                    if grill_session.active {
                        println!("Active Grilling Session:");
                        println!("  Topic:  {}", grill_session.topic);
                        println!("  Output: {}\n", grill_session.out_path.display());
                    } else {
                        println!(
                            "No active grilling session. Start one with /grill \"<topic>\".\n"
                        );
                    }
                }
                Ok(crate::grill::GrillCommand::Reset) => {
                    grill_session = crate::grill::GrillSessionState::default();
                    println!("Active grilling session reset.\n");
                }
                Ok(crate::grill::GrillCommand::Done { out_path }) => {
                    crate::grill::finish_grill(client, &mut grill_session, out_path).await?;
                }
                Ok(crate::grill::GrillCommand::Start { topic, out_path }) => {
                    grill_session = crate::grill::start_grill(client, &topic, out_path).await?;
                }
                Err(e) => {
                    eprintln!("Error: {}\n", e);
                }
            }
            continue;
        }

        if trimmed == "/plan" || trimmed.starts_with("/plan ") {
            let rest = trimmed.strip_prefix("/plan").unwrap().trim();
            let spec = match crate::plan::tokenize_plan_args(rest) {
                Ok(Some(s)) => s,
                Ok(None) => {
                    println!("Plan Commands:");
                    println!(
                        "  /plan \"<task>\" [--out <path>]  - Create an implementation plan (PLAN.md)"
                    );
                    println!(
                        "  --out <path>                   - Destination file (default: PLAN.md)"
                    );
                    println!("  /plan --help                   - Show this help message\n");
                    continue;
                }
                Err(e) => {
                    eprintln!("Error: {}\n", e);
                    continue;
                }
            };

            crate::plan::run_plan(client, spec).await?;
            continue;
        }

        if trimmed == "/goal" || trimmed.starts_with("/goal ") {
            let rest = trimmed.strip_prefix("/goal").unwrap().trim();
            let spec = match crate::goal::tokenize_goal_args(rest) {
                Ok(Some(s)) => s,
                Ok(None) => {
                    println!("Ralph Loop Commands:");
                    println!(
                        "  /goal \"<text>\" [--until \"<cmd|criteria>\"] [--max-iters N] [--promise TAG]"
                    );
                    println!(
                        "                               - Start autonomous Ralph Wiggum goal loop"
                    );
                    println!(
                        "  --until \"<cmd>\"              - Exit 0 shell command (e.g. \"cargo test\")"
                    );
                    println!(
                        "  --until \"<criteria>\"         - Natural language acceptance criteria (e.g. \"has 5 lines\")"
                    );
                    println!("  --max-iters <N>              - Maximum iterations (default 10)");
                    println!(
                        "  --promise <TAG>              - Completion tag (default DONE -> <promise>DONE</promise>)"
                    );
                    println!("  /goal --help                 - Show this help message\n");
                    continue;
                }
                Err(e) => {
                    eprintln!("Error: {}\n", e);
                    continue;
                }
            };

            crate::goal::run_ralph_loop(client, spec).await?;
            continue;
        }

        let prompt_text = if let Some(expanded) = expand_custom_command(&config.commands, trimmed) {
            let cmd_name = trimmed.split_whitespace().next().unwrap_or("")[1..].to_string();
            println!("⚡ Shortcut /{} -> {}\n", cmd_name, truncate_line(&expanded, 80));
            expanded
        } else if trimmed.starts_with('/') && !is_builtin_command(trimmed) {
            eprintln!("Unknown command: {trimmed}. Type '/help' for available commands.\n");
            continue;
        } else {
            trimmed.to_string()
        };

        let response = match client.run_stream(&prompt_text).await {
            Ok(res) => res,
            Err(e) => {
                eprintln!("\n[error] {}\n", e);
                continue;
            }
        };

        let (stats, _) = render_stream_response(response, config.cli.display_options()).await;
        if let Some(line) = stats_line(&stats) {
            print!("\n\n{line}");
            io::stdout().flush().ok();
        }
        println!("\n");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(prompt: i32, output: i32, thinking: i32, total: i32) -> UsageMetadata {
        UsageMetadata {
            prompt_token_count: prompt,
            candidates_token_count: output,
            thinking_token_count: Some(thinking).filter(|count| *count > 0),
            total_token_count: total,
            ..Default::default()
        }
    }

    fn event_with_usage(id: &str, usage: UsageMetadata) -> Event {
        let mut event = Event::with_id(id, "invocation");
        event.llm_response.usage_metadata = Some(usage);
        event
    }

    #[test]
    fn thought_time_sums_each_thinking_phase_and_skips_tool_waits() {
        let mut tracker = TurnTracker::new();
        let start = Instant::now();

        tracker.on_thinking(start);
        tracker.on_output(start + Duration::from_millis(1200));
        // Tool wait time between thinking and output is excluded from thinking duration
        tracker.on_output(start + Duration::from_secs(3));
        tracker.on_thinking(start + Duration::from_secs(4));
        tracker.on_output(start + Duration::from_secs(5));
        tracker.finish(start, start + Duration::from_secs(6));

        assert!(tracker.stats.thinking_seen);
        assert_eq!(tracker.stats.thought, Duration::from_millis(2200));
    }

    #[test]
    fn unfinished_thinking_phase_is_closed_by_finish() {
        let mut tracker = TurnTracker::new();
        let start = Instant::now();
        tracker.on_thinking(start);
        tracker.finish(start, start + Duration::from_millis(500));
        assert_eq!(tracker.stats.thought, Duration::from_millis(500));
    }

    /// Models that send no thinking deltas (e.g. gemini-3.5-flash-lite)
    /// fall back to duration until first output
    #[test]
    fn thought_falls_back_to_time_of_first_output_without_thinking() {
        let mut tracker = TurnTracker::new();
        let start = Instant::now();

        tracker.on_output(start + Duration::from_millis(900));
        tracker.on_output(start + Duration::from_secs(4));
        tracker.finish(start, start + Duration::from_secs(5));

        assert!(tracker.stats.thinking_seen);
        assert_eq!(tracker.stats.thought, Duration::from_millis(900));
    }

    /// Thinking arriving with answer in same event (phase unmeasurable)
    /// falls back to first output time
    #[test]
    fn thought_falls_back_when_thinking_arrives_with_the_answer() {
        let mut tracker = TurnTracker::new();
        let start = Instant::now();

        tracker.on_thinking(start + Duration::from_millis(700));
        tracker.on_output(start + Duration::from_millis(700));
        tracker.finish(start, start + Duration::from_secs(2));

        assert!(tracker.stats.thinking_seen);
        assert_eq!(tracker.stats.thought, Duration::from_millis(700));
    }

    /// No output at all (error from start) must not report turn duration as thinking time
    #[test]
    fn no_output_means_no_thought_time() {
        let mut tracker = TurnTracker::new();
        let start = Instant::now();
        tracker.finish(start, start + Duration::from_secs(3));
        assert!(!tracker.stats.thinking_seen);
        assert_eq!(stats_line(&tracker.stats), None);
    }

    #[test]
    fn usage_is_summed_across_events_and_latest_chunk_wins_per_event() {
        let mut tracker = TurnTracker::new();

        tracker.on_event(&event_with_usage("evt-1", usage(100, 20, 5, 125)));
        // Partial chunks for same model call share event id:
        // latest overwrites earlier value instead of adding
        tracker.on_event(&event_with_usage("evt-1", usage(100, 30, 5, 135)));
        tracker.on_event(&event_with_usage("evt-2", usage(50, 10, 0, 60)));
        tracker.finish(Instant::now(), Instant::now());

        assert_eq!(
            tracker.stats.tokens,
            Some(TokenTotals {
                prompt: 150,
                output: 40,
                thinking: 5,
                total: 195,
            })
        );
    }

    #[test]
    fn no_usage_from_provider_means_no_token_part() {
        let mut tracker = TurnTracker::new();
        tracker.finish(Instant::now(), Instant::now());
        assert!(tracker.stats.tokens.is_none());
        assert_eq!(stats_line(&tracker.stats), None);
    }

    #[test]
    fn stats_line_omits_missing_pieces() {
        let mut stats = TurnStats {
            thought: Duration::from_millis(1840),
            thinking_seen: true,
            tokens: None,
        };
        assert_eq!(stats_line(&stats).as_deref(), Some("💭 thought 1.8s"));

        stats.tokens = Some(TokenTotals {
            prompt: 900,
            output: 160,
            thinking: 180,
            total: 1240,
        });
        assert_eq!(
            stats_line(&stats).as_deref(),
            Some("💭 thought 1.8s · tokens 1,240 (prompt 900 · output 160 · thinking 180)")
        );

        // No actual thinking (provider does not stream thinking) → do not show thought
        let tokens_only = TurnStats {
            thought: Duration::ZERO,
            thinking_seen: false,
            tokens: Some(TokenTotals {
                prompt: 12,
                output: 7,
                thinking: 0,
                total: 19,
            }),
        };
        assert_eq!(
            stats_line(&tokens_only).as_deref(),
            Some("tokens 19 (prompt 12 · output 7)")
        );
    }

    #[test]
    fn stats_line_uses_milliseconds_below_one_second() {
        let stats = TurnStats {
            thought: Duration::from_millis(420),
            thinking_seen: true,
            tokens: None,
        };
        assert_eq!(stats_line(&stats).as_deref(), Some("💭 thought 420ms"));
    }

    #[test]
    fn fmt_int_groups_thousands() {
        assert_eq!(fmt_int(0), "0");
        assert_eq!(fmt_int(999), "999");
        assert_eq!(fmt_int(1240), "1,240");
        assert_eq!(fmt_int(1_048_576), "1,048,576");
        assert_eq!(fmt_int(-1234), "-1,234");
    }

    // ---- Phase 6: tool progress + result rendering ----

    #[test]
    fn format_progress_chunk_flattens_newlines_to_one_line() {
        assert_eq!(
            format_progress_chunk("still rendering — 30s elapsed\n"),
            "… still rendering — 30s elapsed"
        );
        assert_eq!(format_progress_chunk("a\r\nb"), "… a b");
    }

    #[test]
    fn truncate_line_flattens_whitespace_and_cuts_at_char_boundary() {
        assert_eq!(truncate_line("  a\n  b  ", 100), "a b");
        // Cut at max chars + ellipsis (no panic mid-char — slice with chars())
        let long = "x".repeat(300);
        let cut = truncate_line(&long, 10);
        assert_eq!(cut.chars().count(), 11); // 10 + …
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn fmt_bytes_human_readable() {
        assert_eq!(fmt_bytes(0), "0 B");
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1024), "1.0 KB");
        assert_eq!(fmt_bytes(245_000), "239.3 KB");
        assert_eq!(fmt_bytes(1_258_291), "1.2 MB");
    }

    #[test]
    fn tool_result_lines_media_outputs_list_paths_with_size_and_duration() {
        let response = serde_json::json!({
            "status": "ok",
            "outputs": [
                {"path": "media/run-1/video-1.mp4", "bytes": 1_258_291, "duration_seconds": 8.03}
            ],
            "run_dir": "media/run-1"
        });
        let lines = tool_result_lines("generate_video", &response);
        assert_eq!(lines[0], "✅ generate_video");
        assert_eq!(lines[1], "  media/run-1/video-1.mp4 (1.2 MB · 8.0s)");
    }

    #[test]
    fn tool_result_lines_error_short_line() {
        let response = serde_json::json!({"error": "model not found: veo-x"});
        let lines = tool_result_lines("generate_video", &response);
        assert_eq!(lines, vec!["❌ generate_video: model not found: veo-x"]);
    }

    #[test]
    fn tool_result_lines_generic_json_is_truncated_single_line() {
        let response = serde_json::json!({"results": ["a".repeat(500)]});
        let lines = tool_result_lines("search", &response);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("✅ search: "));
        assert!(lines[0].ends_with('…'));
        // Total line length does not exceed prefix + 160 + ellipsis
        assert!(lines[0].chars().count() < "✅ search: ".len() + 162);
    }

    #[test]
    fn tool_result_lines_plain_string_response() {
        let response = serde_json::json!("file saved");
        let lines = tool_result_lines("create_file", &response);
        assert_eq!(lines, vec!["✅ create_file: file saved"]);
    }

    #[test]
    fn test_expand_custom_command_simple() {
        let mut commands = HashMap::new();
        commands.insert("review".to_string(), "Inspect the codebase changes".to_string());

        let expanded = expand_custom_command(&commands, "/review").unwrap();
        assert_eq!(expanded, "Inspect the codebase changes");
    }

    #[test]
    fn test_expand_custom_command_with_args_placeholder() {
        let mut commands = HashMap::new();
        commands.insert("review".to_string(), "Inspect file: {args}".to_string());

        let expanded = expand_custom_command(&commands, "/review src/main.rs").unwrap();
        assert_eq!(expanded, "Inspect file: src/main.rs");
    }

    #[test]
    fn test_expand_custom_command_appends_args_when_no_placeholder() {
        let mut commands = HashMap::new();
        commands.insert("review".to_string(), "Inspect the codebase changes".to_string());

        let expanded = expand_custom_command(&commands, "/review src/main.rs").unwrap();
        assert_eq!(expanded, "Inspect the codebase changes\n\nContext: src/main.rs");
    }

    #[test]
    fn test_expand_custom_command_unknown_returns_none() {
        let commands = HashMap::new();
        assert!(expand_custom_command(&commands, "/unknown").is_none());
        assert!(expand_custom_command(&commands, "hello").is_none());
    }

    #[test]
    fn test_is_builtin_command() {
        assert!(is_builtin_command("/new"));
        assert!(is_builtin_command("/grill"));
        assert!(is_builtin_command("/plan"));
        assert!(is_builtin_command("/goal"));
        assert!(is_builtin_command("exit"));
        assert!(!is_builtin_command("/review"));
    }
}
