use crate::config::{NamiConfig, RemoteMcpServerConfig};
use adk_rust::prelude::Toolset;
use adk_rust::tool::mcp::manager::{McpServerConfig, McpServerManager};
use adk_rust::tool::mcp::{McpAuth, McpHttpClientBuilder, McpToolset};
use std::collections::HashMap;
use std::sync::Arc;

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

    // บน Windows คำสั่งแบบ .cmd (เช่น npx) ต้องระบุนามสกุลไฟล์ให้ชัดเจน
    // มิฉะนั้น tokio::process::Command จะค้นหาไฟล์ไม่พบ
    if cfg!(windows) {
        for (id, config) in manager.all_configs().await {
            let needs_extension = !config.command.contains('.')
                && matches!(config.command.as_str(), "npx" | "npm" | "pnpm" | "yarn");
            if needs_extension {
                let mut replacement = config.clone();
                replacement.command = format!("{}.cmd", config.command);
                if let Err(err) = manager.update_server(&id, replacement).await {
                    tracing::warn!("Unable to adjust command of MCP Server [{}]: {}", id, err);
                }
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
