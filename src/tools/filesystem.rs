use crate::workspace::WorkspaceGuard;
use adk_rust::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct ReadFileParams {
    /// Path to the file to read
    pub path: String,
    /// Maximum number of lines to return (optional)
    pub limit: Option<usize>,
    /// 1-based first line to return (optional)
    pub offset: Option<usize>,
}

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct WriteFileParams {
    /// Path relative to the workspace root.
    pub path: String,
    /// The full file content to write.
    pub content: String,
}

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct ListFilesParams {
    /// Optional sub-directory to search within.
    pub path: Option<String>,
    /// Glob pattern, e.g. '**/*.rs'.
    pub pattern: Option<String>,
}

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct ExecuteCommandParams {
    /// Shell command to execute (e.g. "cargo check", "git status", "cargo test").
    pub command: String,
    /// Optional relative sub-directory path to execute the command in.
    pub path: Option<String>,
}

/// สร้างเครื่องมือ execute_command เดี่ยว ๆ สำหรับใช้ร่วมกับ DevTools โดยไม่เกิดชื่อเครื่องมือซ้ำ
pub fn create_execute_command_tool(guard: WorkspaceGuard) -> Arc<dyn Tool> {
    let exec_tool = FunctionTool::new(
        "execute_command",
        "Execute a terminal shell command inside the workspace and return stdout, stderr, and exit code.",
        move |_ctx, args| {
            let g = guard.clone();
            async move {
                let cmd_str = match args.get("command").and_then(|v| v.as_str()) {
                    Some(c) => c,
                    None => return Ok(json!({ "error": "Missing required argument: command" })),
                };

                let sub_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                let work_dir = match g.validate_path(sub_path, true) {
                    Ok(p) => p,
                    Err(e) => return Ok(json!({ "error": e.to_string() })),
                };

                #[cfg(target_os = "windows")]
                let mut command = {
                    let bin = crate::mcp::resolve_executable("cmd");
                    let mut c = tokio::process::Command::new(bin);
                    c.args(["/C", cmd_str]);
                    c
                };

                #[cfg(not(target_os = "windows"))]
                let mut command = {
                    let bin = crate::mcp::resolve_executable("sh");
                    let mut c = tokio::process::Command::new(bin);
                    c.args(["-c", cmd_str]);
                    c
                };

                command.current_dir(work_dir);
                command.stdout(std::process::Stdio::piped());
                command.stderr(std::process::Stdio::piped());

                let output = match command.output().await {
                    Ok(out) => out,
                    Err(e) => return Ok(json!({ "error": format!("Failed to execute command: {}", e) })),
                };

                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);

                Ok(json!({
                    "success": output.status.success(),
                    "exit_code": exit_code,
                    "stdout": stdout,
                    "stderr": stderr
                }))
            }
        },
    )
    .with_parameters_schema::<ExecuteCommandParams>();

    Arc::new(exec_tool)
}

/// สร้างเครื่องมือ filesystem ที่ประกอบด้วย read_file, write_file, list_files และ execute_command พร้อม WorkspaceGuard
pub fn create_filesystem_toolset(guard: WorkspaceGuard) -> Vec<Arc<dyn Tool>> {
    let g_read = guard.clone();
    let read_tool = FunctionTool::new(
        "read_file",
        "Read a text file from the workspace and return its contents with line numbers. Supports optional offset and limit.",
        move |_ctx, args| {
            let g = g_read.clone();
            async move {
                let path_str = match args.get("path").and_then(|v| v.as_str()) {
                    Some(p) => p,
                    None => return Ok(json!({ "error": "Missing required argument: path" })),
                };

                let target_path = match g.validate_path(path_str, false) {
                    Ok(p) => p,
                    Err(e) => return Ok(json!({ "error": e.to_string() })),
                };

                let content = match tokio::fs::read_to_string(&target_path).await {
                    Ok(c) => c,
                    Err(e) => return Ok(json!({ "error": format!("Failed to read file: {}", e) })),
                };

                let lines: Vec<&str> = content.lines().collect();
                let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let limit = args.get("limit").and_then(|v| v.as_u64()).map(|l| l as usize);

                let start_idx = if offset > 0 { offset - 1 } else { 0 };
                let end_idx = match limit {
                    Some(l) => std::cmp::min(start_idx + l, lines.len()),
                    None => lines.len(),
                };

                let mut formatted = String::new();
                for (i, line) in lines.iter().enumerate().take(end_idx).skip(start_idx) {
                    formatted.push_str(&format!("{:6}\t{}\n", i + 1, line));
                }

                Ok(json!({
                    "path": path_str,
                    "content": formatted,
                    "total_lines": lines.len()
                }))
            }
        },
    )
    .with_parameters_schema::<ReadFileParams>();

    let g_write = guard.clone();
    let write_tool = FunctionTool::new(
        "write_file",
        "Create a new file or overwrite an existing one with the given content. Parent directories are created automatically.",
        move |_ctx, args| {
            let g = g_write.clone();
            async move {
                let path_str = match args.get("path").and_then(|v| v.as_str()) {
                    Some(p) => p,
                    None => return Ok(json!({ "error": "Missing required argument: path" })),
                };
                let content = match args.get("content").and_then(|v| v.as_str()) {
                    Some(c) => c,
                    None => return Ok(json!({ "error": "Missing required argument: content" })),
                };

                let target_path = match g.validate_path(path_str, true) {
                    Ok(p) => p,
                    Err(e) => return Ok(json!({ "error": e.to_string() })),
                };

                if let Some(parent) = target_path.parent()
                    && !parent.exists()
                    && let Err(e) = tokio::fs::create_dir_all(parent).await
                {
                    return Ok(json!({ "error": format!("Failed to create parent directories: {}", e) }));
                }

                if let Err(e) = tokio::fs::write(&target_path, content).await {
                    return Ok(json!({ "error": format!("Failed to write file: {}", e) }));
                }

                Ok(json!({
                    "success": true,
                    "path": path_str,
                    "bytes_written": content.len()
                }))
            }
        },
    )
    .with_parameters_schema::<WriteFileParams>();

    let g_list = guard.clone();
    let list_tool = FunctionTool::new(
        "list_files",
        "List files matching a glob pattern or in a sub-directory.",
        move |_ctx, args| {
            let g = g_list.clone();
            async move {
                let sub_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                let pattern = args
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("**/*");

                let target_dir = match g.validate_path(sub_path, false) {
                    Ok(p) => p,
                    Err(e) => return Ok(json!({ "error": e.to_string() })),
                };

                let full_pattern = if target_dir.is_dir() {
                    format!(
                        "{}/{}",
                        target_dir.to_string_lossy().replace('\\', "/"),
                        pattern
                    )
                } else {
                    pattern.to_string()
                };

                let mut matches = Vec::new();
                for path in glob::glob(&full_pattern).into_iter().flatten().flatten() {
                    if path.is_file() {
                        matches.push(path.to_string_lossy().to_string());
                    }
                }

                Ok(json!({
                    "matches": matches,
                    "count": matches.len()
                }))
            }
        },
    )
    .with_parameters_schema::<ListFilesParams>();

    let exec_tool = create_execute_command_tool(guard);

    vec![
        Arc::new(read_tool),
        Arc::new(write_tool),
        Arc::new(list_tool),
        exec_tool,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_filesystem_toolset_names() {
        let guard = WorkspaceGuard::new(vec![std::path::PathBuf::from("./")], false).unwrap();
        let tools = create_filesystem_toolset(guard);
        let names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
        assert_eq!(names, vec!["read_file", "write_file", "list_files", "execute_command"]);
    }

    #[tokio::test]
    async fn test_execute_command_echo() {
        let guard = WorkspaceGuard::new(vec![std::path::PathBuf::from("./")], false).unwrap();
        let tools = create_filesystem_toolset(guard);
        let exec_tool = tools.into_iter().find(|t| t.name() == "execute_command").unwrap();

        let dummy_ctx = Arc::new(adk_rust::tool::SimpleToolContext::new("test"));
        let res = exec_tool.execute(dummy_ctx, json!({ "command": "echo hello_nami_test" })).await.unwrap();

        assert_eq!(res["success"], true, "Expected success in tool result: {:?}", res);
        assert_eq!(res["exit_code"], 0);
        let stdout = res["stdout"].as_str().unwrap();
        assert!(stdout.contains("hello_nami_test"));
    }

    #[tokio::test]
    async fn test_execute_command_read_only_rejected() {
        let guard = WorkspaceGuard::new(vec![std::path::PathBuf::from("./")], true).unwrap();
        let tools = create_filesystem_toolset(guard);
        let exec_tool = tools.into_iter().find(|t| t.name() == "execute_command").unwrap();

        let dummy_ctx = Arc::new(adk_rust::tool::SimpleToolContext::new("test"));
        let res = exec_tool.execute(dummy_ctx, json!({ "command": "echo hello" })).await.unwrap();

        assert!(res.get("error").is_some());
        let err = res["error"].as_str().unwrap();
        assert!(err.contains("Read-Only mode"));
    }
}
