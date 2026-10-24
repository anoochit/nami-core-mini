//! Ralph Wiggum Autonomous Loop Engine for Nami
//!
//! Implements the Ralph Wiggum technique:
//! - Fresh, clean context on every iteration (no context rot, no model-turn ordering bugs).
//! - State lives in the workspace files and git history.
//! - Dual-verification: deterministic shell command check OR natural language LLM evaluator check.
//! - Self-directed completion: agent outputs `<promise>DONE</promise>`.

use crate::api_client::NamiApiClient;
use std::time::Duration;

/// Mode of completion verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UntilMode {
    /// Deterministic shell command: exit code 0 means success.
    Command(String),
    /// Natural language acceptance criteria or expected result text.
    NaturalLanguage(String),
    /// No `--until` specified; relies on agent completion promise `<promise>DONE</promise>`.
    None,
}

/// Specification for a Ralph Wiggum goal loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalSpec {
    pub goal: String,
    pub until: UntilMode,
    pub max_iterations: u32,
    pub promise_tag: String,
}

impl Default for GoalSpec {
    fn default() -> Self {
        Self {
            goal: String::new(),
            until: UntilMode::None,
            max_iterations: 10,
            promise_tag: "DONE".to_string(),
        }
    }
}

/// Outcome of running an external command check.
#[derive(Debug, Clone)]
pub struct CheckOutcome {
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub feedback: String,
}

/// Detects whether the `--until` text is a shell command or natural language.
pub fn detect_until_mode(until_text: Option<&str>) -> UntilMode {
    let Some(raw) = until_text else {
        return UntilMode::None;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return UntilMode::None;
    }

    // Explicit prefix overrides
    if let Some(cmd) = trimmed.strip_prefix("cmd:") {
        return UntilMode::Command(cmd.trim().to_string());
    }
    if let Some(nl) = trimmed
        .strip_prefix("eval:")
        .or_else(|| trimmed.strip_prefix("text:"))
    {
        return UntilMode::NaturalLanguage(nl.trim().to_string());
    }

    // Check for shell pipeline/redirection symbols
    if trimmed.contains('|')
        || trimmed.contains("&&")
        || trimmed.contains("||")
        || trimmed.contains('>')
        || trimmed.contains('<')
        || trimmed.contains("$(")
        || trimmed.contains('`')
    {
        return UntilMode::Command(trimmed.to_string());
    }

    // Check if starts with a path prefix (e.g. ./test.sh, /bin/true)
    if trimmed.starts_with("./")
        || trimmed.starts_with(".\\")
        || trimmed.starts_with('/')
        || (trimmed.len() > 3
            && trimmed.chars().nth(1) == Some(':')
            && (trimmed.chars().nth(2) == Some('\\') || trimmed.chars().nth(2) == Some('/')))
    {
        return UntilMode::Command(trimmed.to_string());
    }

    // Check first word against known executable commands
    let first_word = trimmed
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();
    let first_word =
        first_word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');

    const KNOWN_COMMANDS: &[&str] = &[
        "cargo",
        "npm",
        "pnpm",
        "yarn",
        "bun",
        "python",
        "python3",
        "py",
        "pytest",
        "git",
        "test",
        "grep",
        "cat",
        "ls",
        "dir",
        "echo",
        "find",
        "make",
        "go",
        "deno",
        "node",
        "sh",
        "bash",
        "powershell",
        "pwsh",
        "cmd",
        "curl",
        "jq",
        "gcc",
        "g++",
        "rustc",
        "docker",
        "docker-compose",
        "mvn",
        "gradle",
        "mix",
        "dotnet",
        "swift",
        "tsc",
        "eslint",
        "prettier",
        "diff",
        "head",
        "tail",
        "wc",
        "awk",
        "sed",
    ];

    if KNOWN_COMMANDS.contains(&first_word) {
        return UntilMode::Command(trimmed.to_string());
    }

    // Default to NaturalLanguage / Acceptance Criteria
    UntilMode::NaturalLanguage(trimmed.to_string())
}

/// Tokenizes a command line respecting quotes.
pub fn tokenize_goal_args(input: &str) -> anyhow::Result<Option<GoalSpec>> {
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

    let mut goal_text = String::new();
    let mut until_text: Option<String> = None;
    let mut max_iters: u32 = 10;
    let mut promise_tag: String = "DONE".to_string();

    let mut idx = 0;
    while idx < tokens.len() {
        let tok = &tokens[idx];
        if tok == "--until" {
            idx += 1;
            if idx >= tokens.len() {
                anyhow::bail!("Missing value for --until");
            }
            until_text = Some(tokens[idx].clone());
        } else if let Some(val) = tok.strip_prefix("--until=") {
            until_text = Some(val.to_string());
        } else if tok == "--max-iters" {
            idx += 1;
            if idx >= tokens.len() {
                anyhow::bail!("Missing value for --max-iters");
            }
            max_iters = tokens[idx]
                .parse::<u32>()
                .map_err(|_| anyhow::anyhow!("Invalid number for --max-iters: {}", tokens[idx]))?;
        } else if let Some(val) = tok.strip_prefix("--max-iters=") {
            max_iters = val
                .parse::<u32>()
                .map_err(|_| anyhow::anyhow!("Invalid number for --max-iters: {}", val))?;
        } else if tok == "--promise" {
            idx += 1;
            if idx >= tokens.len() {
                anyhow::bail!("Missing value for --promise");
            }
            promise_tag = tokens[idx].clone();
        } else if let Some(val) = tok.strip_prefix("--promise=") {
            promise_tag = val.to_string();
        } else if tok.starts_with('-') {
            anyhow::bail!("Unknown option: {}", tok);
        } else if goal_text.is_empty() {
            goal_text = tok.clone();
        } else {
            // Append extra positional words to goal
            goal_text.push(' ');
            goal_text.push_str(tok);
        }
        idx += 1;
    }

    if goal_text.is_empty() {
        anyhow::bail!("Missing goal prompt. Usage: /goal \"<goal>\" [--until \"<cmd|text>\"]");
    }

    if max_iters < 1 {
        anyhow::bail!("--max-iters must be at least 1");
    }

    let until = detect_until_mode(until_text.as_deref());

    Ok(Some(GoalSpec {
        goal: goal_text,
        until,
        max_iterations: max_iters,
        promise_tag,
    }))
}

/// Constructs the prompt for an iteration of the Ralph loop.
pub fn build_ralph_prompt(
    spec: &GoalSpec,
    iteration: u32,
    max_iterations: u32,
    feedback: Option<&str>,
) -> String {
    let mut prompt = String::new();

    prompt.push_str(&format!(
        "# Ralph Wiggum Autonomous Loop (Iteration {}/{})\n\n",
        iteration, max_iterations
    ));
    prompt.push_str("You are an autonomous agent operating in a Ralph Wiggum development loop.\n");
    prompt.push_str("Each iteration starts with a clean, fresh context. The filesystem, git history, and workspace files are your persistent memory.\n\n");

    prompt.push_str("## Goal\n");
    prompt.push_str(&spec.goal);
    prompt.push_str("\n\n");

    match &spec.until {
        UntilMode::Command(cmd) => {
            prompt.push_str("## Verification Check\n");
            prompt.push_str(&format!(
                "After this turn, the system runs this shell command: `{}`\n",
                cmd
            ));
            prompt.push_str("Your changes must make this check exit with code 0.\n\n");
        }
        UntilMode::NaturalLanguage(criteria) => {
            prompt.push_str("## Acceptance Criteria\n");
            prompt.push_str(&format!("{}\n\n", criteria));
            prompt.push_str("An independent evaluator will inspect the workspace files against this criteria.\n\n");
        }
        UntilMode::None => {}
    }

    prompt.push_str("## Instructions for this Iteration\n");
    prompt.push_str("1. **Inspect**: Use tools to inspect current files, git diff, or test outputs to see previous progress.\n");
    prompt.push_str(
        "2. **Execute**: Make concrete progress on ONE focused step or fix toward the goal.\n",
    );
    prompt.push_str("3. **Verify**: Test your changes using available tools.\n");
    prompt.push_str(&format!(
        "4. **Completion**: If and ONLY if the entire goal is fully completed and verified, output `<promise>{}</promise>` at the end of your response. If more work remains, summarize your progress and what is next.\n\n",
        spec.promise_tag
    ));

    if let Some(fb) = feedback {
        prompt.push_str("## Feedback from Previous Iteration (Address this now)\n");
        prompt.push_str(fb);
        prompt.push_str("\n\n");
    }

    prompt
}

/// Constructs the prompt for an independent LLM Evaluator turn.
pub fn build_evaluator_prompt(goal: &str, criteria: &str) -> String {
    format!(
        r#"You are an objective verification auditor inspecting the workspace.
Your task is to determine whether the user's acceptance criteria has been met.

Goal: {goal}
Acceptance Criteria: {criteria}

Instructions:
1. Use your filesystem, file read, or search tools to inspect the current state of workspace files.
2. Check strictly whether the acceptance criteria is satisfied.
3. Do not modify any files. Only read and verify.
4. Respond in this exact format:

If satisfied:
<verified>YES</verified>
Explanation: <brief explanation of why it is satisfied>

If NOT satisfied:
<verified>NO</verified>
Missing or failing:
- <concise bullet point of what is missing or incorrect>
"#
    )
}

/// Checks if `<promise>TAG</promise>` (case-insensitive) is present in the text.
pub fn extract_promise(text: &str, tag: &str) -> bool {
    let lower_text = text.to_lowercase();
    let open_tag = format!("<promise>{}", tag.to_lowercase());
    if let Some(idx) = lower_text.find(&open_tag) {
        let rest = &lower_text[idx + open_tag.len()..];
        return rest.starts_with("</promise>")
            || rest.starts_with(' ')
            || rest.contains("</promise>");
    }
    false
}

/// Extracts verification verdict from evaluator response.
/// Returns `(is_passed, explanation_or_missing)`.
pub fn extract_verification(text: &str) -> (bool, String) {
    let upper = text.to_uppercase();
    if upper.contains("<VERIFIED>YES</VERIFIED>") || upper.contains("<VERIFIED> YES </VERIFIED>") {
        let reason = text
            .lines()
            .find(|l| {
                l.to_lowercase().starts_with("explanation:")
                    || l.to_lowercase().starts_with("reason:")
            })
            .map(|l| l.to_string())
            .unwrap_or_else(|| "Acceptance criteria verified.".to_string());
        (true, reason)
    } else {
        // Collect missing points or entire feedback
        let missing = text
            .lines()
            .filter(|l| !l.to_uppercase().contains("<VERIFIED>") && !l.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let feedback = if missing.is_empty() {
            "Acceptance criteria not satisfied.".to_string()
        } else {
            missing
        };
        (false, feedback)
    }
}

/// Runs a shell verification command asynchronously with a timeout.
pub async fn run_command_check(cmd: &str, timeout_secs: u64) -> CheckOutcome {
    #[cfg(target_os = "windows")]
    let mut command = {
        let bin = crate::mcp::resolve_executable("cmd");
        let mut c = tokio::process::Command::new(bin);
        c.args(["/C", cmd]);
        c
    };

    #[cfg(not(target_os = "windows"))]
    let mut command = {
        let bin = crate::mcp::resolve_executable("sh");
        let mut c = tokio::process::Command::new(bin);
        c.args(["-c", cmd]);
        c
    };

    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let run_fut = async {
        let child = command.spawn()?;
        child.wait_with_output().await
    };

    match tokio::time::timeout(Duration::from_secs(timeout_secs), run_fut).await {
        Ok(Ok(output)) => {
            let passed = output.status.success();
            let code = output.status.code();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{}{}", stdout, stderr);
            let tail = combined.lines().rev().take(15).collect::<Vec<_>>();
            let mut tail_rev = tail;
            tail_rev.reverse();
            let feedback = tail_rev.join("\n");

            CheckOutcome {
                passed,
                exit_code: code,
                feedback: if feedback.is_empty() {
                    format!("Command exited with code {:?}", code)
                } else {
                    feedback
                },
            }
        }
        Ok(Err(e)) => CheckOutcome {
            passed: false,
            exit_code: None,
            feedback: format!("Failed to spawn check command: {}", e),
        },
        Err(_) => CheckOutcome {
            passed: false,
            exit_code: None,
            feedback: format!("Check command timed out after {} seconds", timeout_secs),
        },
    }
}

/// Runs the Ralph Wiggum autonomous loop with Ctrl-C interruption protection.
pub async fn run_ralph_loop(client: &mut NamiApiClient, spec: GoalSpec) -> anyhow::Result<()> {
    tokio::select! {
        res = run_ralph_loop_inner(client, spec) => res,
        _ = tokio::signal::ctrl_c() => {
            println!("\n\n[ralph] Interrupted (Ctrl-C). Stopping loop. Workspace changes preserved.\n");
            let _ = client.new_session().await;
            Ok(())
        }
    }
}

async fn run_ralph_loop_inner(client: &mut NamiApiClient, spec: GoalSpec) -> anyhow::Result<()> {
    println!();
    println!("  [ralph] Starting Ralph Wiggum Autonomous Loop");
    println!("  Goal:          {}", spec.goal);
    match &spec.until {
        UntilMode::Command(cmd) => println!("  Verification:  Shell Command -> `{}`", cmd),
        UntilMode::NaturalLanguage(criteria) => {
            println!(
                "  Verification:  Evaluator Acceptance Criteria -> \"{}\"",
                criteria
            )
        }
        UntilMode::None => println!(
            "  Verification:  Agent Completion Promise (<promise>{}</promise>)",
            spec.promise_tag
        ),
    }
    println!("  Max Iters:     {}", spec.max_iterations);
    println!();

    let mut previous_feedback: Option<String> = None;
    let mut completed = false;

    for iteration in 1..=spec.max_iterations {
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!(
            "  [ralph] Iteration {}/{} (Fresh Context)",
            iteration, spec.max_iterations
        );
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

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
        if let Some(line) = crate::cli::stats_line(&stats) {
            println!("\n{line}");
        }
        println!();

        let promise_found = extract_promise(&answer, &spec.promise_tag);

        // 4. Verify completion condition
        match &spec.until {
            UntilMode::Command(cmd) => {
                println!("[ralph] Running verification check: `{}`...", cmd);
                let outcome = run_command_check(cmd, 120).await;
                if outcome.passed {
                    println!(
                        "  [check {}/{}] exit 0 (passed!)",
                        iteration, spec.max_iterations
                    );
                    println!(
                        "\n🎉 [ralph] Goal completed successfully in {} iteration(s)!\n",
                        iteration
                    );
                    completed = true;
                    break;
                } else {
                    let code_str = outcome
                        .exit_code
                        .map_or_else(|| "none".to_string(), |c| c.to_string());
                    println!(
                        "  [check {}/{}] exit {} (failed)",
                        iteration, spec.max_iterations, code_str
                    );
                    previous_feedback = Some(format!(
                        "Verification command `{}` failed with exit code {}:\n```\n{}\n```",
                        cmd, code_str, outcome.feedback
                    ));
                }
            }
            UntilMode::NaturalLanguage(criteria) => {
                println!(
                    "[evaluator] Inspecting workspace against acceptance criteria: \"{}\"...",
                    criteria
                );
                // Create a separate fresh session for the evaluator turn
                client.new_session().await?;
                let eval_prompt = build_evaluator_prompt(&spec.goal, criteria);
                let eval_resp = client.run_stream(&eval_prompt).await?;
                let (_, eval_answer) = crate::cli::render_stream_response(eval_resp, false).await;
                let (passed, explanation) = extract_verification(&eval_answer);

                if passed {
                    println!("  [evaluator] {}", explanation);
                    println!(
                        "\n🎉 [ralph] Goal completed and verified in {} iteration(s)!\n",
                        iteration
                    );
                    completed = true;
                    break;
                } else {
                    println!("  [evaluator] Criteria not satisfied:\n{}", explanation);
                    previous_feedback = Some(format!(
                        "The independent verification auditor inspected the workspace and reported:\n{}",
                        explanation
                    ));
                }
            }
            UntilMode::None => {
                if promise_found {
                    println!(
                        "\n🎉 [ralph] Goal completed (promise tag fulfilled) in {} iteration(s)!\n",
                        iteration
                    );
                    completed = true;
                    break;
                } else {
                    println!(
                        "  [ralph] No completion promise detected. Advancing to next iteration."
                    );
                    previous_feedback = Some(format!(
                        "You did not output the completion promise tag `<promise>{}</promise>`. Continue working until the goal is completely finished.",
                        spec.promise_tag
                    ));
                }
            }
        }
    }

    if !completed {
        println!(
            "\n⚠️  [ralph] Reached maximum iterations ({}) without satisfying completion condition.",
            spec.max_iterations
        );
        println!(
            "    Your progress is preserved in the workspace files. You can run /goal again to continue.\n"
        );
    }

    // Reset client to a fresh session for returning to chat REPL
    client.new_session().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_until_mode() {
        assert_eq!(detect_until_mode(None), UntilMode::None);
        assert_eq!(detect_until_mode(Some("")), UntilMode::None);

        // Command detection
        assert_eq!(
            detect_until_mode(Some("cargo test")),
            UntilMode::Command("cargo test".into())
        );
        assert_eq!(
            detect_until_mode(Some("npm test -- --watch=false")),
            UntilMode::Command("npm test -- --watch=false".into())
        );
        assert_eq!(
            detect_until_mode(Some("grep -q 'hello' file.txt")),
            UntilMode::Command("grep -q 'hello' file.txt".into())
        );
        assert_eq!(
            detect_until_mode(Some("test -f wiki/hello.md")),
            UntilMode::Command("test -f wiki/hello.md".into())
        );
        assert_eq!(
            detect_until_mode(Some("./run_tests.sh")),
            UntilMode::Command("./run_tests.sh".into())
        );

        // Pipeline / shell redirection
        assert_eq!(
            detect_until_mode(Some("cat file | wc -l")),
            UntilMode::Command("cat file | wc -l".into())
        );

        // Explicit prefixes
        assert_eq!(
            detect_until_mode(Some("cmd:custom_tool check")),
            UntilMode::Command("custom_tool check".into())
        );
        assert_eq!(
            detect_until_mode(Some("eval:has 5 lines")),
            UntilMode::NaturalLanguage("has 5 lines".into())
        );

        // Natural language detection
        assert_eq!(
            detect_until_mode(Some("has 5 line 'hello world'")),
            UntilMode::NaturalLanguage("has 5 line 'hello world'".into())
        );
        assert_eq!(
            detect_until_mode(Some("wiki/hello.md contains 5 lines of hello world")),
            UntilMode::NaturalLanguage("wiki/hello.md contains 5 lines of hello world".into())
        );
    }

    #[test]
    fn test_tokenize_goal_args_simple() {
        let spec = tokenize_goal_args(r#""update wiki" --until "has 5 lines" --max-iters 5"#)
            .unwrap()
            .unwrap();
        assert_eq!(spec.goal, "update wiki");
        assert_eq!(spec.until, UntilMode::NaturalLanguage("has 5 lines".into()));
        assert_eq!(spec.max_iterations, 5);
        assert_eq!(spec.promise_tag, "DONE");
    }

    #[test]
    fn test_tokenize_goal_args_cmd() {
        let spec = tokenize_goal_args(r#"fix bug --until "cargo test" --promise COMPLETED"#)
            .unwrap()
            .unwrap();
        assert_eq!(spec.goal, "fix bug");
        assert_eq!(spec.until, UntilMode::Command("cargo test".into()));
        assert_eq!(spec.max_iterations, 10);
        assert_eq!(spec.promise_tag, "COMPLETED");
    }

    #[test]
    fn test_tokenize_help_returns_none() {
        assert!(tokenize_goal_args("").unwrap().is_none());
        assert!(tokenize_goal_args("--help").unwrap().is_none());
        assert!(tokenize_goal_args("-h").unwrap().is_none());
    }

    #[test]
    fn test_extract_promise() {
        assert!(extract_promise(
            "All work is done! <promise>DONE</promise>",
            "DONE"
        ));
        assert!(extract_promise("<promise>done</promise>", "DONE"));
        assert!(extract_promise("<promise>COMPLETED</promise>", "COMPLETED"));
        assert!(!extract_promise("Still working on it", "DONE"));
    }

    #[test]
    fn test_extract_verification() {
        let passed_text = "<verified>YES</verified>\nExplanation: The file wiki/hello.md has 5 lines of hello world.";
        let (passed, reason) = extract_verification(passed_text);
        assert!(passed);
        assert!(reason.contains("Explanation:"));

        let failed_text =
            "<verified>NO</verified>\nMissing or failing:\n- Only 3 lines found, expected 5.";
        let (passed, feedback) = extract_verification(failed_text);
        assert!(!passed);
        assert!(feedback.contains("Only 3 lines found"));
    }

    #[tokio::test]
    async fn test_run_command_check_echo() {
        #[cfg(target_os = "windows")]
        let outcome = run_command_check("echo hello", 10).await;
        #[cfg(not(target_os = "windows"))]
        let outcome = run_command_check("echo hello", 10).await;

        assert!(outcome.passed);
        assert_eq!(outcome.exit_code, Some(0));
    }
}
