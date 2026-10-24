use crate::config::{AgentTool, NamiConfig};
use crate::mcp::load_mcp_manager;
use crate::tools::{
    create_datetime_tool, create_execute_command_tool, create_filesystem_toolset,
    create_media_toolset, create_search_tool, create_weather_tool,
};
use crate::workspace::WorkspaceGuard;
use adk_devtools::{DevToolset, Workspace};
use adk_retry_reflect::RetryReflectPluginBuilder;
use adk_rust::prelude::*;
use adk_rust::tool::mcp::manager::McpServerManager;
use std::sync::Arc;
use std::time::Duration;

/// Agent creation result: the agent instance itself and the `McpServerManager` handle (if active).
///
/// The handle is returned so `main` can invoke `shutdown()` prior to application exit,
/// preventing the `McpServerManager` from being dropped while servers are active.
pub struct NamiAgent {
    pub agent: Arc<dyn Agent>,
    pub mcp_manager: Option<Arc<McpServerManager>>,
}

/// Sampling temperature to send for `model`, or `None` when it must be omitted.
///
/// `adk-gemini` rejects sampling parameters outright for a few model ids and reports the
/// rejection per request, not at build time — so those ids skip temperature here rather
/// than fail every turn.
pub(crate) fn effective_temperature(config: &NamiConfig, model: &str) -> Option<f32> {
    let temperature = config.temperature?;
    let id = model.rsplit('/').next().unwrap_or(model);
    if matches!(id, "gemini-3.6-flash" | "gemini-3.7-flash") {
        eprintln!("⚠️  temperature is ignored for {id} — that model rejects sampling parameters");
        return None;
    }
    Some(temperature)
}

pub async fn build_nami_agent(
    config: &NamiConfig,
    _guard: WorkspaceGuard,
) -> anyhow::Result<NamiAgent> {
    let model = build_model(config)?;

    // 1. Configure Workspace DevTools according to nami.json (reused by `coder` sub-agent)
    let dev_toolset = build_dev_toolset(config);

    // 2. Retry & Reflect plugin — intercepts tool failures and injects reflection prompts
    //    so the model can self-correct on the next turn instead of failing outright.
    //    Shared across all agents (root + sub-agents): retries are counted per invocation
    //    and transferring to a sub-agent remains in the same invocation, sharing budget correctly.
    let retry_reflect_plugin: Arc<dyn adk_rust::plugin::EnhancedPlugin> = Arc::new(
        RetryReflectPluginBuilder::new()
            .max_retries(3)
            .backoff_exponential(Duration::from_millis(100))
            .max_backoff(Duration::from_secs(10))
            .build()
            .map_err(|e| anyhow::anyhow!("invalid retry-reflect config: {e}"))?,
    );

    // 3. System instruction for root + delegation guidance for specialists (generated from `agents`)
    //    adk-agent selects matching skills and injects them into user turn (not system instruction)
    //    so it bypasses `inject_session_state` — skill bodies with `{id}` like `POST /runs/{id}/wake`
    //    will not trigger "State variable 'id' not found" errors.
    let system_instruction = {
        let base = config.system_instruction.clone();
        match root_delegation_guidance(config) {
            Some(guide) => format!("{base}\n\n{guide}"),
            None => base,
        }
    };

    // 4. Load and manage MCP Servers (stdio) from `mcpServers` in nami.json
    let mcp_manager = load_mcp_manager(config.stdio_servers()).await;
    // Hold handle so `main` can call `shutdown()` on exit
    let mcp_manager_handle = mcp_manager.clone();

    //    Remote MCP Servers (remote/streamable-HTTP) from `mcpServers` (with `url`)
    let mcp_remote_toolsets = crate::mcp::load_mcp_remote_toolsets(config).await;

    // 5. Create Sub-Agents (hierarchical) from `agents` in nami.json
    //
    // `LlmAgentBuilder::sub_agent()` attaches `Arc<dyn Agent>` and adk-rust automatically
    // injects the `transfer_to_agent` tool when at least one sub-agent is present.
    let mut sub_agents: Vec<Arc<dyn Agent>> = Vec::new();
    for spec in config.enabled_agents() {
        // Sub-agent model: use override if specified, otherwise inherit root model
        let sub_model = match &spec.model {
            Some(override_model) => build_model_with(config, Some(override_model))?,
            None => model.clone(),
        };

        let mut sub_builder = LlmAgentBuilder::new(&spec.name)
            .description(spec.effective_description())
            .instruction(&spec.instruction)
            .model(sub_model)
            // Sub-agents load skills from `.skills/` — adk-skill injects only matching skills
            .with_auto_skills()?
            .enhanced_plugin(retry_reflect_plugin.clone());

        let sub_model_name = spec.model.as_deref().unwrap_or(&config.model);
        if let Some(temperature) = effective_temperature(config, sub_model_name) {
            sub_builder = sub_builder.temperature(temperature);
        }

        // Each media tool invocation writes to its own file (concurrency-safe) — enable
        // parallel execution for sub-agents with media tools so multiple assets in one response
        // can be generated concurrently.
        let has_media_tools = spec.tools.contains(&AgentTool::Media);

        // Attach built-in tools per allowlist in config
        let workspace_guard = WorkspaceGuard::new(
            config.workspace.allowed_paths.clone(),
            config.workspace.read_only,
        )?;
        for tool in &spec.tools {
            sub_builder = match tool {
                AgentTool::Search => sub_builder.tool(create_search_tool()),
                AgentTool::Devtools => sub_builder.toolset(Arc::new(build_dev_toolset(config))),
                AgentTool::Filesystem => {
                    let fs_tools = create_filesystem_toolset(workspace_guard.clone());
                    let has_devtools = spec.tools.contains(&AgentTool::Devtools);
                    let mut b = sub_builder;
                    for t in fs_tools {
                        if has_devtools && matches!(t.name(), "read_file" | "write_file") {
                            continue;
                        }
                        b = b.tool(t);
                    }
                    b
                }
                AgentTool::Datetime => sub_builder.tool(create_datetime_tool()),
                AgentTool::Weather => sub_builder.tool(create_weather_tool()),
                // Media tools (Gemini image/TTS/video) — built from `media.*` in nami.json
                // Returns empty vec when `media.enabled` is false (warned below)
                AgentTool::Media => {
                    let media_tools = create_media_toolset(config);
                    if media_tools.is_empty() {
                        eprintln!(
                            "⚠️  Sub-agent `{}` requests `media` tools but `media.enabled` is false in nami.json — skipping",
                            spec.name
                        );
                    }
                    let mut b = sub_builder;
                    for t in media_tools {
                        b = b.tool(t);
                    }
                    b
                }
            };
        }

        if has_media_tools {
            sub_builder = sub_builder
                .tool_execution_strategy(adk_rust::ToolExecutionStrategy::Parallel)
                // Default runner tool timeout is 300s (DEFAULT_TOOL_TIMEOUT), but
                // `media.timeout_secs` = 600s — without extending this, video rendering
                // would be timed out prematurely by the runner before the tool timeout.
                .tool_timeout(std::time::Duration::from_secs(
                    config.media.timeout_secs + 60,
                ));
        }

        sub_agents.push(Arc::new(sub_builder.build()?) as Arc<dyn Agent>);
    }

    // 6. Build LlmAgent (root) and register toolsets (DevTools, MCP Manager), Custom Tools,
    //    and created sub-agents.
    //
    // `with_auto_skills()` loads skill index from `.skills/`
    // `instruction_provider` returns raw text of system instruction without passing through
    // `inject_session_state`, which interprets `{...}` in `instruction` as state variables.
    let mut builder = LlmAgentBuilder::new(&config.name)
        .description("Nami Assistant")
        .instruction_provider(Box::new(move |_ctx: Arc<dyn adk_rust::ReadonlyContext>| {
            let text = system_instruction.clone();
            Box::pin(async move { Ok(text) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = adk_rust::Result<String>> + Send>,
                >
        }))
        .model(model)
        // Load skills from `.skills/` in current directory
        .with_auto_skills()?
        .enhanced_plugin(retry_reflect_plugin.clone())
        .toolset(Arc::new(dev_toolset))
        .tool(create_datetime_tool())
        .tool(create_weather_tool())
        .tool(create_search_tool())
        .tool(create_execute_command_tool(WorkspaceGuard::new(
            config.workspace.allowed_paths.clone(),
            config.workspace.read_only,
        )?));

    if let Some(temperature) = effective_temperature(config, &config.model) {
        builder = builder.temperature(temperature);
    }

    for sub_agent in sub_agents {
        builder = builder.sub_agent(sub_agent);
    }

    if let Some(mcp) = mcp_manager {
        builder = builder.toolset(mcp);
    }

    for remote in mcp_remote_toolsets {
        builder = builder.toolset(remote);
    }

    let agent = builder.build()?;

    Ok(NamiAgent {
        agent: Arc::new(agent),
        mcp_manager: mcp_manager_handle,
    })
}

/// Build `Llm` based on `config.provider` and `config.model` — shared by main agent and Context Compaction summarizer
pub fn build_model(config: &NamiConfig) -> anyhow::Result<Arc<dyn Llm>> {
    build_model_with(config, None)
}

/// Build `Llm` based on `config.provider` — allows overriding `config.model` with `model_override`
///
/// Sub-agents that specify `model` in config will be constructed using this function.
pub fn build_model_with(
    config: &NamiConfig,
    model_override: Option<&str>,
) -> anyhow::Result<Arc<dyn Llm>> {
    let model_name = model_override.unwrap_or(&config.model);
    let model: Arc<dyn Llm> = match config.provider.to_lowercase().as_str() {
        "openai" => {
            let api_key =
                std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "mock_key".to_string());
            if api_key.is_empty() || api_key == "mock_key" {
                eprintln!("\n⚠️  Warning: OPENAI_API_KEY not found in .env or the environment");
                eprintln!("👉 Add OPENAI_API_KEY to .env to use the OpenAI provider\n");
            }
            Arc::new(OpenAIClient::new(OpenAIConfig::new(&api_key, model_name))?)
        }
        "anthropic" => {
            let api_key =
                std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| "mock_key".to_string());
            if api_key.is_empty() || api_key == "mock_key" {
                eprintln!("\n⚠️  Warning: ANTHROPIC_API_KEY not found in .env or the environment");
                eprintln!("👉 Add ANTHROPIC_API_KEY to .env to use the Anthropic provider\n");
            }
            Arc::new(AnthropicClient::new(AnthropicConfig::new(
                &api_key, model_name,
            ))?)
        }
        "ollama" => {
            use adk_rust::model::ollama::{OllamaConfig, OllamaModel};
            Arc::new(OllamaModel::new(OllamaConfig::new(model_name))?)
        }
        "openrouter" => {
            let api_key =
                std::env::var("OPENROUTER_API_KEY").unwrap_or_else(|_| "mock_key".to_string());
            if api_key.is_empty() || api_key == "mock_key" {
                eprintln!("\n⚠️  Warning: OPENROUTER_API_KEY not found in .env or the environment");
                eprintln!("👉 Add OPENROUTER_API_KEY to .env to use the OpenRouter provider\n");
            }
            Arc::new(OpenRouterClient::new(OpenRouterConfig::new(
                &api_key, model_name,
            ))?)
        }
        _ => {
            // Default to Gemini Provider
            let api_key = std::env::var("GOOGLE_API_KEY").unwrap_or_default();
            if api_key.is_empty() || api_key == "mock_key" || api_key == "your_gemini_api_key_here"
            {
                eprintln!("\n⚠️  Warning: GOOGLE_API_KEY not found in .env or the environment");
                eprintln!("👉 Create .env from .env.example and add a real API key, e.g.:");
                eprintln!("   cp .env.example .env\n");
            }
            Arc::new(GeminiModel::new(&api_key, model_name)?)
        }
    };
    Ok(model)
}

/// Build Workspace DevTools from `config.workspace` (file read/write + bash based on `read_only`)
///
/// Separated into helper because it is called twice: by root agent and by sub-agent `coder`
fn build_dev_toolset(config: &NamiConfig) -> DevToolset {
    let workspace_root = config
        .workspace
        .allowed_paths
        .first()
        .cloned()
        .unwrap_or_else(|| std::path::PathBuf::from("./"));

    let mut ws = if config.workspace.read_only {
        Workspace::read_only(&workspace_root)
    } else {
        Workspace::new(&workspace_root)
    };
    ws = ws.allow_bash(true);
    DevToolset::new(ws)
}

/// Delegation guidance appended to root system instruction listing available sub-agents
///
/// Generated from `agents` key in nami.json — names and descriptions match those registered with
/// `.sub_agent()`. Returns `None` when no sub-agent is enabled.
fn root_delegation_guidance(config: &NamiConfig) -> Option<String> {
    let mut agents = config.enabled_agents();
    let first = agents.next()?;

    let mut guide = String::from(
        "You are the main Nami assistant and orchestrator. You have specialist sub-agents \
         you can hand work to via the transfer_to_agent tool:\n",
    );
    for spec in std::iter::once(first).chain(agents) {
        guide.push_str(&format!(
            "- `{}`: {}\n",
            spec.name,
            spec.effective_description()
        ));
    }
    guide.push_str(
        "Handle simple conversational answers yourself. When a task clearly matches a \
         specialist, transfer to it and then summarize its result for the user.",
    );
    Some(guide)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// System instruction with example code `{id}` (like in a skill body)
    /// must not cause run failure. The default template interprets `{id}` as a session
    /// state variable and errors "State variable 'id' not found" — so `instruction_provider`
    /// must be used to return raw text without template injection.
    #[tokio::test]
    async fn test_instruction_with_braces_via_provider() {
        use adk_session::SessionService;

        let config = NamiConfig::load_default().unwrap();

        // Simulate instruction with placeholder-like text similar to skill body
        let system_instruction = format!(
            "{}\n\n[Skill: example]\nExample endpoint: POST /runs/{{id}}/wake",
            config.system_instruction
        );
        assert!(
            system_instruction.contains("{id}"),
            "expected a placeholder-like example in the instruction"
        );

        let model = build_model(&config).unwrap();
        let instruction = system_instruction.clone();
        let test_agent = LlmAgentBuilder::new("test_agent")
            .instruction_provider(Box::new(move |_ctx: Arc<dyn adk_rust::ReadonlyContext>| {
                let text = instruction.clone();
                Box::pin(async move { Ok(text) })
                    as std::pin::Pin<
                        Box<dyn std::future::Future<Output = adk_rust::Result<String>> + Send>,
                    >
            }))
            // Bypass model call so tests do not rely on an external API key
            .before_model_callback(Box::new(|_ctx, _req| {
                Box::pin(async move {
                    Ok(adk_rust::BeforeModelResult::Skip(
                        adk_rust::LlmResponse::new(
                            adk_rust::Content::new("model").with_text("mock reply"),
                        ),
                    ))
                })
            }))
            .model(model)
            .build()
            .unwrap();

        let session_service: Arc<dyn SessionService> =
            Arc::new(adk_session::InMemorySessionService::new());
        let session = session_service
            .create(adk_session::CreateRequest {
                app_name: "test".into(),
                user_id: "user".into(),
                session_id: None,
                state: std::collections::HashMap::new(),
            })
            .await
            .unwrap();

        let runner = adk_rust::runner::Runner::builder()
            .app_name("test")
            .agent(Arc::new(test_agent))
            .session_service(session_service)
            .build()
            .unwrap();

        use futures::StreamExt;
        let mut stream = runner
            .run(
                adk_rust::UserId::new("user").unwrap(),
                adk_rust::SessionId::new(session.id()).unwrap(),
                adk_rust::Content::new("user").with_text("hi"),
            )
            .await
            .unwrap();

        let mut got_model_text = false;
        while let Some(evt) = stream.next().await {
            match evt {
                Ok(e) => {
                    if let Some(content) = e.content()
                        && content.role == "model"
                        && content.parts.iter().any(
                            |p| matches!(p, adk_rust::Part::Text { text } if text == "mock reply"),
                        )
                    {
                        got_model_text = true;
                    }
                }
                Err(err) => panic!("Run error: {err}"),
            }
        }
        assert!(got_model_text, "expected the mocked model reply to arrive");
    }

    /// Guidance must list name + description of all enabled sub-agents and exclude disabled ones
    #[test]
    fn test_root_delegation_guidance_lists_enabled_agents() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" },
            "agents": [
                { "name": "researcher", "description": "research things", "instruction": "r" },
                { "name": "coder", "description": "write code", "instruction": "c" },
                { "name": "off", "instruction": "o", "disabled": true }
            ]
        }"#;
        let config: NamiConfig = serde_json::from_str(json).unwrap();

        let guidance = root_delegation_guidance(&config).expect("agents present");
        assert!(guidance.contains("researcher"));
        assert!(guidance.contains("research things"));
        assert!(guidance.contains("coder"));
        assert!(guidance.contains("write code"));
        assert!(
            !guidance.contains("`off`"),
            "disabled agent must not be listed"
        );
    }

    /// No sub-agent (or all disabled) must return `None` — root does not append guidance or inject transfer
    #[test]
    fn test_root_delegation_guidance_none_when_no_agents() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" }
        }"#;
        let config: NamiConfig = serde_json::from_str(json).unwrap();
        assert!(root_delegation_guidance(&config).is_none());
    }

    /// Models that reject sampling parameters must not receive temperature
    #[test]
    fn test_effective_temperature_skips_rejecting_models() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s", "temperature": 0.2,
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" }
        }"#;
        let config: NamiConfig = serde_json::from_str(json).unwrap();

        assert_eq!(
            effective_temperature(&config, "gemini-3.5-flash-lite"),
            Some(0.2)
        );
        assert_eq!(effective_temperature(&config, "gemini-3.7-flash"), None);
        assert_eq!(
            effective_temperature(&config, "models/gemini-3.6-flash"),
            None
        );
    }

    /// Unspecified `temperature` in nami.json → `None` (model uses provider default)
    #[test]
    fn test_effective_temperature_absent_is_none() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" }
        }"#;
        let config: NamiConfig = serde_json::from_str(json).unwrap();

        assert_eq!(
            effective_temperature(&config, "gemini-3.5-flash-lite"),
            None
        );
    }

    /// Tool progress must stream from `ToolContext::emit_progress` through the runner
    /// stream to the client. Model is mocked (no API calls); first turn returns FunctionCall,
    /// second turn returns final message.
    #[tokio::test]
    async fn test_tool_progress_flows_through_runner_stream() {
        use adk_session::SessionService;

        // Mock tool that emits progress twice during execution
        let progress_tool = FunctionTool::new(
            "slow_tool",
            "test tool that emits progress while running",
            |ctx: Arc<dyn ToolContext>, _args: serde_json::Value| {
                Box::pin(async move {
                    ctx.emit_progress("stdout", "step 1: preparing...\n").await;
                    ctx.emit_progress("stdout", "step 2: finished").await;
                    Ok(serde_json::json!({"status": "ok"}))
                })
            },
        );

        // Mock model: first call = FunctionCall("slow_tool"), subsequent calls = "done"
        let model_call = adk_rust::Content {
            role: "model".to_string(),
            parts: vec![adk_rust::Part::FunctionCall {
                name: "slow_tool".to_string(),
                args: serde_json::json!({}),
                id: Some("call-1".to_string()),
                thought_signature: None,
            }],
        };
        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let call_count = counter.clone();

        let config = NamiConfig::load_default().unwrap();
        let model = build_model(&config).unwrap();
        let test_agent = LlmAgentBuilder::new("progress_agent")
            .instruction_provider(Box::new(|_ctx: Arc<dyn adk_rust::ReadonlyContext>| {
                let text = String::from("You are a test agent.");
                Box::pin(async move { Ok(text) })
                    as std::pin::Pin<
                        Box<dyn std::future::Future<Output = adk_rust::Result<String>> + Send>,
                    >
            }))
            .before_model_callback(Box::new(move |_ctx, _req| {
                let n = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let response = if n == 0 {
                    adk_rust::LlmResponse::new(model_call.clone())
                } else {
                    adk_rust::LlmResponse::new(adk_rust::Content::new("model").with_text("done"))
                };
                Box::pin(async move { Ok(adk_rust::BeforeModelResult::Skip(response)) })
            }))
            .tool(Arc::new(progress_tool))
            .model(model)
            .build()
            .unwrap();

        let session_service: Arc<dyn SessionService> =
            Arc::new(adk_session::InMemorySessionService::new());
        let session = session_service
            .create(adk_session::CreateRequest {
                app_name: "test".into(),
                user_id: "user".into(),
                session_id: None,
                state: std::collections::HashMap::new(),
            })
            .await
            .unwrap();

        let runner = adk_rust::runner::Runner::builder()
            .app_name("test")
            .agent(Arc::new(test_agent))
            .session_service(session_service)
            .build()
            .unwrap();

        use futures::StreamExt;
        let mut stream = runner
            .run(
                adk_rust::UserId::new("user").unwrap(),
                adk_rust::SessionId::new(session.id()).unwrap(),
                adk_rust::Content::new("user").with_text("run the tool"),
            )
            .await
            .unwrap();

        let mut progress_chunks: Vec<String> = Vec::new();
        let mut tool_response_seen = false;
        while let Some(evt) = stream.next().await {
            let evt = evt.unwrap_or_else(|err| panic!("Run error: {err}"));
            if evt.tool_progress_stream().is_some()
                && let Some(content) = evt.content()
            {
                for part in &content.parts {
                    if let adk_rust::Part::Text { text } = part {
                        progress_chunks.push(text.clone());
                    }
                }
            }
            if let Some(content) = evt.content() {
                for part in &content.parts {
                    if let adk_rust::Part::FunctionResponse {
                        function_response, ..
                    } = part
                        && function_response.name == "slow_tool"
                    {
                        tool_response_seen = true;
                    }
                }
            }
        }

        assert!(
            progress_chunks.iter().any(|c| c.contains("step 1")),
            "expected progress chunks (stdout) in the run stream, got {progress_chunks:?}"
        );
        assert!(
            tool_response_seen,
            "expected a FunctionResponse event for slow_tool"
        );
    }
}
