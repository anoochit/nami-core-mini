//! Implementation Plan Generator for Nami
//!
//! Generates a structured, actionable implementation plan (`PLAN.md`) by exploring
//! the workspace architecture, affected files, and decomposition into atomic steps
//! with ready-to-run `/goal` commands.

use crate::api_client::NamiApiClient;
use std::path::PathBuf;

/// Specification for generating an implementation plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanSpec {
    pub task: String,
    pub out_path: PathBuf,
}

impl Default for PlanSpec {
    fn default() -> Self {
        Self {
            task: String::new(),
            out_path: PathBuf::from("PLAN.md"),
        }
    }
}

/// Tokenizes arguments for `/plan "<task>" [--out <path>]`.
pub fn tokenize_plan_args(input: &str) -> anyhow::Result<Option<PlanSpec>> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed == "--help" || trimmed == "-h" || trimmed == "help" {
        return Ok(None);
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

    let mut task_parts = Vec::new();
    let mut out_path = PathBuf::from("PLAN.md");

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
            task_parts.push(tok.clone());
        }
        idx += 1;
    }

    if task_parts.is_empty() {
        anyhow::bail!("Missing task description. Usage: /plan \"<task>\" [--out <path>]");
    }

    Ok(Some(PlanSpec {
        task: task_parts.join(" "),
        out_path,
    }))
}

/// Constructs the prompt for the Senior Software Architect to create an implementation plan.
pub fn build_plan_prompt(task: &str, grill_context: Option<&str>) -> String {
    let mut prompt = String::new();

    prompt.push_str("# Implementation Plan Request\n\n");
    prompt.push_str("You are an experienced Principal Software Architect. Your job is to create a rigorous, actionable implementation plan.\n\n");

    prompt.push_str("## Target Task / Feature\n");
    prompt.push_str(task);
    prompt.push_str("\n\n");

    if let Some(grill) = grill_context {
        prompt.push_str("## Grilling Interview Context (GRILL.md)\n");
        prompt.push_str(grill);
        prompt.push_str("\n\n");
    }

    prompt.push_str("## Instructions\n");
    prompt.push_str("1. **Inspect Workspace**: First, use your tools (`list_dir`, `read_file`, `search`) to inspect existing source files, `Cargo.toml`, and architectural conventions.\n");
    prompt.push_str("2. **Decompose Into Atomic Steps**: Break the implementation into small, cohesive, testable steps.\n");
    prompt.push_str("3. **Provide /goal Commands**: For every step, include a concrete `/goal` command with an `--until` check (e.g. `cargo check`, unit test, or natural language check).\n");
    prompt.push_str("4. **Format Output**: Format your response cleanly using this exact Markdown template:\n\n");

    prompt.push_str(
        r#"```markdown
# Implementation Plan: <Task Title>

## 1. Objective & Scope
- What is being built
- Non-goals / Out of scope

## 2. Current State & Affected Files
- Files to modify: `src/...`
- Files to create: `src/...`
- Dependencies / Crates: (if any)

## 3. Step-by-Step Implementation Steps
- [ ] **Step 1: [Name]**
  - Details: [Description of exact changes]
  - Files: `src/...`
  - Verification: [Test or check]
  - Command: `/goal "<description>" --until "<check>"`

- [ ] **Step 2: [Name]**
  - Details: ...
  - Command: `/goal "..." --until "..."`

## 4. Verification & Testing Strategy
- Commands to run for full regression testing
- Edge cases and error scenarios covered
```"#,
    );

    prompt.push_str("\n\nDo not start writing code yet. Deliver a complete, high-quality implementation plan.\n");
    prompt
}

/// Cleans markdown plan text if wrapped in triple backticks.
pub fn clean_plan_markdown(raw: &str) -> String {
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

/// Runs the `/plan` command in chat mode.
pub async fn run_plan(client: &mut NamiApiClient, spec: PlanSpec) -> anyhow::Result<()> {
    tokio::select! {
        res = run_plan_inner(client, spec) => res,
        _ = tokio::signal::ctrl_c() => {
            println!("\n\n[plan] Interrupted (Ctrl-C). Returning to chat.\n");
            let _ = client.new_session().await;
            Ok(())
        }
    }
}

async fn run_plan_inner(client: &mut NamiApiClient, spec: PlanSpec) -> anyhow::Result<()> {
    println!();
    println!("  [plan] Generating Implementation Plan");
    println!("  Task:     {}", spec.task);
    println!("  Output:   {}", spec.out_path.display());
    println!();

    // Check if GRILL.md exists in current working directory
    let grill_context = tokio::fs::read_to_string("GRILL.md").await.ok();
    if grill_context.is_some() {
        println!("  [plan] Incorporating existing GRILL.md decisions.");
    }

    // Always start a fresh session for planning to keep clean context
    client.new_session().await?;

    let prompt = build_plan_prompt(&spec.task, grill_context.as_deref());
    let response = client.run_stream(&prompt).await?;
    let (stats, answer) = crate::cli::render_stream_response(response, true).await;

    if let Some(line) = crate::cli::stats_line(&stats) {
        println!("\n{line}");
    }
    println!();

    // Save the generated plan to out_path
    let cleaned = clean_plan_markdown(&answer);
    if !cleaned.is_empty() {
        if let Some(parent) = spec.out_path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        match tokio::fs::write(&spec.out_path, &cleaned).await {
            Ok(_) => {
                println!(
                    "📄 [plan] Implementation plan saved to: {}\n",
                    spec.out_path.display()
                );
            }
            Err(e) => {
                eprintln!(
                    "⚠️  [plan] Could not write {}: {}\n",
                    spec.out_path.display(),
                    e
                );
            }
        }
    }

    // Reset client to fresh session for returning to chat
    client.new_session().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_plan_args_simple() {
        let spec = tokenize_plan_args(r#""Add JWT authentication""#)
            .unwrap()
            .unwrap();
        assert_eq!(spec.task, "Add JWT authentication");
        assert_eq!(spec.out_path, PathBuf::from("PLAN.md"));
    }

    #[test]
    fn test_tokenize_plan_args_with_out() {
        let spec = tokenize_plan_args(r#"Refactor storage --out "docs/plan.md""#)
            .unwrap()
            .unwrap();
        assert_eq!(spec.task, "Refactor storage");
        assert_eq!(spec.out_path, PathBuf::from("docs/plan.md"));
    }

    #[test]
    fn test_tokenize_plan_args_with_equals() {
        let spec = tokenize_plan_args(r#"--out=custom_plan.md Fix memory leak"#)
            .unwrap()
            .unwrap();
        assert_eq!(spec.task, "Fix memory leak");
        assert_eq!(spec.out_path, PathBuf::from("custom_plan.md"));
    }

    #[test]
    fn test_tokenize_plan_help_returns_none() {
        assert!(tokenize_plan_args("").unwrap().is_none());
        assert!(tokenize_plan_args("--help").unwrap().is_none());
        assert!(tokenize_plan_args("-h").unwrap().is_none());
    }

    #[test]
    fn test_clean_plan_markdown() {
        let raw = "```markdown\n# Implementation Plan\n\n- Step 1\n```";
        let cleaned = clean_plan_markdown(raw);
        assert_eq!(cleaned, "# Implementation Plan\n\n- Step 1");
    }
}
