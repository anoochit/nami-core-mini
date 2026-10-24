//! Grilling Interview Engine for Nami
//!
//! Conducts a rigorous architectural interview before planning or coding:
//! - Surfaces ambiguities and questions assumptions.
//! - Checks codebase first to avoid asking redundant questions.
//! - Provides clear recommendations for each load-bearing decision.
//! - Compiles decisions into `GRILL.md` for handoff to `/plan`.

use crate::api_client::NamiApiClient;
use std::path::PathBuf;

/// Commands supported by the `/grill` CLI interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrillCommand {
    Help,
    Start { topic: String, out_path: PathBuf },
    Done { out_path: Option<PathBuf> },
    Status,
    Reset,
}

/// State of an active grilling session in the REPL.
#[derive(Debug, Clone)]
pub struct GrillSessionState {
    pub active: bool,
    pub topic: String,
    pub out_path: PathBuf,
}

impl Default for GrillSessionState {
    fn default() -> Self {
        Self {
            active: false,
            topic: String::new(),
            out_path: PathBuf::from("GRILL.md"),
        }
    }
}

/// Tokenizes `/grill` arguments.
pub fn tokenize_grill_args(input: &str) -> anyhow::Result<GrillCommand> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed == "--help" || trimmed == "-h" || trimmed == "help" {
        return Ok(GrillCommand::Help);
    }

    if trimmed == "status" {
        return Ok(GrillCommand::Status);
    }
    if trimmed == "reset" || trimmed == "clear" {
        return Ok(GrillCommand::Reset);
    }
    if trimmed == "done" || trimmed == "finish" || trimmed == "save" {
        return Ok(GrillCommand::Done { out_path: None });
    }
    if let Some(rest) = trimmed
        .strip_prefix("done ")
        .or_else(|| trimmed.strip_prefix("save "))
    {
        let rest = rest.trim();
        let out_path = if let Some(p) = rest.strip_prefix("--out=") {
            Some(PathBuf::from(p.trim().trim_matches('"')))
        } else if let Some(p) = rest.strip_prefix("--out ") {
            Some(PathBuf::from(p.trim().trim_matches('"')))
        } else {
            Some(PathBuf::from(rest.trim_matches('"')))
        };
        return Ok(GrillCommand::Done { out_path });
    }

    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    let mut chars = trimmed.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '"' | '\'' => {
                if let Some(q) = in_quote {
                    if q == c {
                        in_quote = None;
                    } else {
                        current.push(c);
                    }
                } else {
                    in_quote = Some(c);
                }
            }
            ' ' | '\t' if in_quote.is_none() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => {
                current.push(c);
            }
        }
    }
    if let Some(q) = in_quote {
        anyhow::bail!("Unterminated quote: missing closing {}", q);
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    let mut topic_parts = Vec::new();
    let mut out_path = PathBuf::from("GRILL.md");

    let mut idx = 0;
    while idx < tokens.len() {
        let tok = &tokens[idx];
        if tok == "--out" {
            idx += 1;
            if idx >= tokens.len() {
                anyhow::bail!("Missing value for --out");
            }
            out_path = PathBuf::from(&tokens[idx]);
        } else if let Some(val) = tok.strip_prefix("--out=") {
            out_path = PathBuf::from(val);
        } else if tok.starts_with('-') {
            anyhow::bail!("Unknown option: {}", tok);
        } else {
            topic_parts.push(tok.clone());
        }
        idx += 1;
    }

    if topic_parts.is_empty() {
        anyhow::bail!("Missing topic for grilling interview. Usage: /grill \"<topic>\"");
    }

    Ok(GrillCommand::Start {
        topic: topic_parts.join(" "),
        out_path,
    })
}

/// Constructs the opening prompt for a grilling interview.
pub fn build_grill_start_prompt(topic: &str) -> String {
    format!(
        r#"# Grilling Interview: {topic}

You are an expert Principal Software Architect conducting an intensive "Grilling Interview" before any code or implementation plan is written.
Your mission is to drag all ambiguities into the light, question hidden assumptions, and force decisions on every load-bearing branch of the design tree.

## Rules of Engagement:
1. **Inspect Codebase First**: Use your filesystem and search tools (`list_dir`, `read_file`, `search`) to inspect existing models, dependencies in `Cargo.toml`, and architectural conventions. If a question can be answered from existing code, do NOT ask the user.
2. **One Critical Decision at a Time**: Focus on the most load-bearing decision first. Do not overwhelm with 10 questions at once.
3. **Always Recommend**: Provide a concrete recommendation based on current codebase architecture and explain why.
4. **Structured Format**:
   Format your response like this:

   ### Decision 1: [Topic/Branch]
   - **Context**: Why this decision is load-bearing.
   - **Recommendation**: [Your recommended choice with rationale].
   - **Options**:
     (a) [Option A] (Recommended)
     (b) [Option B]
     (c) Something else (explain)

Conclude by asking the user for their preference (or custom choice), and remind them:
"Type `/grill done` when all decisions are resolved to synthesize `GRILL.md`."
"#
    )
}

/// Constructs the summary synthesis prompt for `/grill done`.
pub fn build_grill_summary_prompt(topic: &str) -> String {
    format!(
        r#"# Synthesize Grilling Interview Decisions: {topic}

Please review our conversation so far and synthesize all decisions, trade-offs, and agreed constraints into a clean, comprehensive `GRILL.md` document.

Format your output cleanly using this Markdown structure:

```markdown
# Grilling Specification: {topic}

## 1. Goal & Boundaries
- **In-Scope**: [What we are building]
- **Non-Goals / Out-of-Scope**: [What we are explicitly NOT doing]

## 2. Agreed Architectural Decisions & Technical Locks
- **Decision 1**: [Choice + Rationale]
- **Decision 2**: [Choice + Rationale]

## 3. Edge Cases & Failure Handling Strategy
- [How unexpected errors, network/disk/validation failures are handled]

## 4. Observable Success Criteria
- [How we objectively know it is working]

## 5. Next Step
Run `/plan "{topic}"` to generate the step-by-step implementation plan.
```
"#
    )
}

/// Cleans markdown summary text if wrapped in triple backticks.
pub fn clean_grill_markdown(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(stripped) = trimmed.strip_prefix("```markdown")
        && let Some(end) = stripped.rfind("```")
    {
        return stripped[..end].trim().to_string();
    }
    if let Some(stripped) = trimmed.strip_prefix("```")
        && let Some(end) = stripped.rfind("```")
    {
        return stripped[..end].trim().to_string();
    }
    trimmed.to_string()
}

/// Starts a new grilling session.
pub async fn start_grill(
    client: &mut NamiApiClient,
    topic: &str,
    out_path: PathBuf,
) -> anyhow::Result<GrillSessionState> {
    println!();
    println!("  [grill] Starting Grilling Interview");
    println!("  Topic:     {}", topic);
    println!("  Output:    {}", out_path.display());
    println!();

    // Start fresh session for the interview
    client.new_session().await?;

    let prompt = build_grill_start_prompt(topic);
    let response = client.run_stream(&prompt).await?;
    let (stats, _) = crate::cli::render_stream_response(
        response,
        crate::config::DisplayOptions::all_enabled(),
    )
    .await;

    if let Some(line) = crate::cli::stats_line(&stats) {
        println!("\n{line}");
    }
    println!();

    Ok(GrillSessionState {
        active: true,
        topic: topic.to_string(),
        out_path,
    })
}

/// Finishes the grilling interview and writes the summary to disk.
pub async fn finish_grill(
    client: &mut NamiApiClient,
    session: &mut GrillSessionState,
    custom_out: Option<PathBuf>,
) -> anyhow::Result<()> {
    if !session.active {
        println!("⚠️  [grill] No active grilling session. Start one with: /grill \"<topic>\"\n");
        return Ok(());
    }

    let out_path = custom_out.unwrap_or_else(|| session.out_path.clone());

    println!();
    println!(
        "  [grill] Synthesizing Decisions into {}",
        out_path.display()
    );
    println!();

    let prompt = build_grill_summary_prompt(&session.topic);
    let response = client.run_stream(&prompt).await?;
    let (stats, answer) = crate::cli::render_stream_response(
        response,
        crate::config::DisplayOptions::all_enabled(),
    )
    .await;

    if let Some(line) = crate::cli::stats_line(&stats) {
        println!("\n{line}");
    }
    println!();

    let cleaned = clean_grill_markdown(&answer);
    if !cleaned.is_empty() {
        if let Some(parent) = out_path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        match tokio::fs::write(&out_path, &cleaned).await {
            Ok(_) => {
                println!("📄 [grill] Decisions saved to: {}", out_path.display());
                println!(
                    "👉 Next step: Run `/plan \"{}\"` to generate an implementation plan!\n",
                    session.topic
                );
            }
            Err(e) => {
                eprintln!(
                    "⚠️  [grill] Could not write {}: {}\n",
                    out_path.display(),
                    e
                );
            }
        }
    }

    session.active = false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_grill_args_start() {
        let cmd = tokenize_grill_args(r#""Add rate limiting" --out "GRILL_SPEC.md""#).unwrap();
        assert_eq!(
            cmd,
            GrillCommand::Start {
                topic: "Add rate limiting".into(),
                out_path: PathBuf::from("GRILL_SPEC.md"),
            }
        );
    }

    #[test]
    fn test_tokenize_grill_args_done() {
        assert_eq!(
            tokenize_grill_args("done").unwrap(),
            GrillCommand::Done { out_path: None }
        );
        assert_eq!(
            tokenize_grill_args("done --out custom.md").unwrap(),
            GrillCommand::Done {
                out_path: Some(PathBuf::from("custom.md")),
            }
        );
    }

    #[test]
    fn test_tokenize_grill_args_status_reset_help() {
        assert_eq!(tokenize_grill_args("status").unwrap(), GrillCommand::Status);
        assert_eq!(tokenize_grill_args("reset").unwrap(), GrillCommand::Reset);
        assert_eq!(tokenize_grill_args("clear").unwrap(), GrillCommand::Reset);
        assert_eq!(tokenize_grill_args("").unwrap(), GrillCommand::Help);
        assert_eq!(tokenize_grill_args("--help").unwrap(), GrillCommand::Help);
    }

    #[test]
    fn test_clean_grill_markdown() {
        let raw = "```markdown\n# Grilling Specification\n\nContent\n```";
        let cleaned = clean_grill_markdown(raw);
        assert_eq!(cleaned, "# Grilling Specification\n\nContent");
    }
}
