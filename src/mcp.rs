use crate::config::{NamiConfig, RemoteMcpServerConfig};
use adk_rust::prelude::Toolset;
use adk_rust::tool::mcp::manager::{McpServerConfig, McpServerManager};
use adk_rust::tool::mcp::{McpAuth, McpHttpClientBuilder, McpToolset};
use std::collections::HashMap;
use std::sync::Arc;

/// ค้นหาและตรวจสอบ path ของ executable แบบ cross-platform (Windows, Linux, macOS)
///
/// ป้องกันปัญหา `program not found` เมื่อรัน subprocess Tool / MCP / DevTools:
/// - Windows: ค้นหาใน PATH ร่วมกับ PATHEXT (เช่น `.exe`, `.cmd`, `.bat`) หากไม่ระบุนามสกุล
/// - Linux/macOS: ค้นหาใน PATH และจัดการ fallback สำหรับคำสั่งยอดนิยม เช่น `python` -> `python3`
pub fn resolve_executable(cmd: &str) -> String {
    let path = std::path::Path::new(cmd);
    // ถ้าระบุ absolute path หรือ path ที่มี separator อยู่แล้ว ให้ใช้ค่าเดิม
    if path.is_absolute() || cmd.contains('/') || cmd.contains('\\') {
        return cmd.to_string();
    }

    if cfg!(windows) {
        let has_ext = path.extension().is_some();
        let pathext_var =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let extensions: Vec<&str> = if has_ext {
            vec![""]
        } else {
            pathext_var.split(';').collect()
        };

        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                for ext in &extensions {
                    let file_name = if ext.is_empty() {
                        cmd.to_string()
                    } else if ext.starts_with('.') {
                        format!("{}{}", cmd, ext)
                    } else {
                        format!("{}.{}", cmd, ext)
                    };
                    let candidate = dir.join(&file_name);
                    if candidate.is_file() {
                        return candidate.to_string_lossy().into_owned();
                    }
                }
            }
        }

        // Fallback บน Windows สำหรับ CLI tools ของ Node/Python หากหาใน PATH ไม่พบ
        if !has_ext
            && matches!(
                cmd,
                "npx" | "npm" | "pnpm" | "yarn" | "corepack" | "tsx" | "bunx" | "uvx"
            )
        {
            return format!("{}.cmd", cmd);
        }
    } else {
        // Linux & macOS: ค้นหาใน PATH
        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let candidate = dir.join(cmd);
                if candidate.is_file() {
                    return candidate.to_string_lossy().into_owned();
                }
            }
        }

        // Fallback สำหรับ Unix: ถ้าเรียก `python` แล้วไม่พบ ให้ลองหา `python3`
        if cmd == "python"
            && let Some(path_var) = std::env::var_os("PATH")
        {
            for dir in std::env::split_paths(&path_var) {
                let candidate = dir.join("python3");
                if candidate.is_file() {
                    return candidate.to_string_lossy().into_owned();
                }
            }
        }
    }

    cmd.to_string()
}

/// โหลดและเริ่ม MCP Servers แบบ stdio (มี `command`) จากคีย์ `mcpServers`
///
/// คืนค่า `Some(manager)` เมื่อมี stdio server และสร้าง Manager สำเร็จ
/// มิฉะนั้นจะคืนค่า `None` เพื่อให้ Agent ทำงานได้ต่อโดยไม่มี MCP Toolset
pub async fn load_mcp_manager(
    servers: HashMap<String, McpServerConfig>,
) -> Option<Arc<McpServerManager>> {
    if servers.is_empty() {
        return None;
    }

    // สร้าง JSON ชั่วคราวในรูป mcp.json เพื่อใช้ `from_json`
    // (คงการ validate server ID ให้ตรงกับการอ่านไฟล์แบบเดิม)
    let json = serde_json::json!({ "mcpServers": servers }).to_string();
    let manager = match McpServerManager::from_json(&json) {
        Ok(manager) => Arc::new(manager),
        Err(err) => {
            tracing::warn!("Unable to load MCP configuration: {}", err);
            return None;
        }
    };

    // ปรับแก้ command ของ MCP Server แต่ละตัวให้รองรับ cross-platform (Windows, Linux, macOS)
    for (id, config) in manager.all_configs().await {
        let resolved = resolve_executable(&config.command);
        if resolved != config.command {
            let mut replacement = config.clone();
            replacement.command = resolved;
            if let Err(err) = manager.update_server(&id, replacement).await {
                tracing::warn!("Unable to adjust command of MCP Server [{}]: {}", id, err);
            }
        }
    }

    let outcomes = manager.start_all().await;
    for (id, outcome) in outcomes {
        match outcome {
            Ok(_) => tracing::info!("Started MCP Server [{}] successfully", id),
            Err(err) => tracing::warn!("MCP Server [{}] failed to start: {}", id, err),
        }
    }

    manager.start_monitoring();

    Some(manager)
}

/// โหลดและเชื่อมต่อ MCP Servers ระยะไกล (มี `url`) จากคีย์ `mcpServers`
///
/// ต่างจาก stdio ที่จัดการผ่าน `McpServerManager` ตัวนี้เชื่อมต่อโดยตรงผ่าน
/// streamable HTTP และคืนเฉพาะ server ที่เชื่อมต่อสำเร็จ — server ที่ล้มเหลว
/// ถูกข้ามพร้อมบันทึก warning เพื่อให้ agent ทำงานต่อได้ตามปกติ
pub async fn load_mcp_remote_toolsets(config: &NamiConfig) -> Vec<Arc<dyn Toolset>> {
    let mut toolsets: Vec<Arc<dyn Toolset>> = Vec::new();
    for (id, server) in config.remote_servers() {
        if server.disabled {
            tracing::info!("MCP remote server [{id}] is disabled; skipping");
            continue;
        }
        match connect_remote_server(&id, &server).await {
            Ok(toolset) => {
                tracing::info!("Connected MCP remote server [{id}] at {}", server.url);
                toolsets.push(Arc::new(toolset));
            }
            Err(err) => {
                tracing::warn!(
                    "MCP remote server [{id}] failed to connect ({}): {err}",
                    server.url
                );
            }
        }
    }
    toolsets
}

/// เปิดการเชื่อมต่อเดียวไปยัง MCP server ระยะไกล และตั้งชื่อ toolset ตาม server id
async fn connect_remote_server(
    id: &str,
    server: &RemoteMcpServerConfig,
) -> anyhow::Result<McpToolset<()>> {
    let mut builder = McpHttpClientBuilder::new(server.url.clone());
    for (name, value) in &server.headers {
        builder = builder.header(name.clone(), value.clone());
    }
    if let Some(token) = &server.bearer_token {
        builder = builder.with_auth(McpAuth::bearer(token.clone()));
    }
    Ok(builder.connect().await?.with_name(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_executable_preserves_absolute_paths() {
        let abs = if cfg!(windows) {
            "C:\\bin\\tool.exe"
        } else {
            "/usr/bin/tool"
        };
        assert_eq!(resolve_executable(abs), abs);
    }

    #[test]
    fn test_resolve_executable_resolves_common_cmd() {
        let res = resolve_executable("npx");
        assert!(!res.is_empty());
        if cfg!(windows) {
            assert!(res.to_lowercase().ends_with(".cmd") || res.to_lowercase().ends_with(".exe"));
        }
    }
}
