use adk_rust::tool::mcp::McpServerConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamiConfig {
    pub name: String,
    pub version: String,
    pub provider: String,
    pub model: String,
    pub system_instruction: String,
    pub temperature: Option<f32>,
    pub workspace: WorkspaceConfig,
    pub storage: StorageConfig,
    /// Custom slash command prompt shortcuts (e.g. `"review": "Inspect codebase and run cargo test"`)
    #[serde(default)]
    pub commands: HashMap<String, String>,
    /// HTTP Server configuration
    #[serde(default)]
    pub server: ServerOptionsConfig,
    /// CLI client configuration
    #[serde(default)]
    pub cli: CliOptionsConfig,
    /// MCP Servers (stdio and remote) under `mcpServers` key (mcp.json format)
    /// Entries with `command` are stdio (managed by `McpServerManager`)
    /// Entries with `url` are remote/streamable-HTTP (connected directly)
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: HashMap<String, McpServerEntry>,
    /// Context Compaction configuration (post-run, turn-count based)
    #[serde(default)]
    pub compaction: NamiCompactionConfig,
    /// Hierarchical Sub-Agents — root delegates specialist tasks via `transfer_to_agent`
    /// Each entry is constructed as `LlmAgent` and attached as a sub-agent of root
    #[serde(default)]
    pub agents: Vec<SubAgentConfig>,
    /// Media generation configuration (image/TTS/video via Gemini API)
    #[serde(default)]
    pub media: MediaConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerOptionsConfig {
    #[serde(default = "default_server_host")]
    pub host: String,
    #[serde(default = "default_server_port")]
    pub port: u16,
}

fn default_server_host() -> String {
    "127.0.0.1".to_string()
}

fn default_server_port() -> u16 {
    8080
}

impl Default for ServerOptionsConfig {
    fn default() -> Self {
        Self {
            host: default_server_host(),
            port: default_server_port(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliOptionsConfig {
    #[serde(default = "default_cli_user_id")]
    pub user_id: String,
    #[serde(default = "default_display_true", alias = "thinking", alias = "show_thinking")]
    pub show_thinking: bool,
    #[serde(default = "default_display_true", alias = "tool_call", alias = "tool_calls", alias = "show_tool_calls", alias = "show_tool_call")]
    pub show_tool_call: bool,
    #[serde(default = "default_display_true", alias = "tool_result", alias = "tool_results", alias = "show_tool_results", alias = "show_tool_result")]
    pub show_tool_result: bool,
    #[serde(default = "default_display_true", alias = "answer", alias = "show_answer", alias = "show_answer_label", alias = "answer_label")]
    pub show_answer: bool,
    #[serde(default)]
    pub display: Option<DisplayOptions>,
}

fn default_cli_user_id() -> String {
    "user".to_string()
}

fn default_display_true() -> bool {
    true
}

impl Default for CliOptionsConfig {
    fn default() -> Self {
        Self {
            user_id: default_cli_user_id(),
            show_thinking: true,
            show_tool_call: true,
            show_tool_result: true,
            show_answer: true,
            display: None,
        }
    }
}

impl CliOptionsConfig {
    pub fn display_options(&self) -> DisplayOptions {
        if let Some(d) = self.display {
            d
        } else {
            DisplayOptions {
                show_thinking: self.show_thinking,
                show_tool_call: self.show_tool_call,
                show_tool_result: self.show_tool_result,
                show_answer: self.show_answer,
            }
        }
    }
}

/// CLI / REPL display options (thinking, tool call, tool result, answer label)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayOptions {
    /// Show model thinking process (`🤔 thinking`)
    #[serde(default = "default_display_true", alias = "thinking", alias = "show_thinking")]
    pub show_thinking: bool,
    /// Show tool invocations (`🔧 name(args)`)
    #[serde(default = "default_display_true", alias = "tool_call", alias = "tool_calls", alias = "show_tool_calls", alias = "show_tool_call")]
    pub show_tool_call: bool,
    /// Show tool results (`✅ name` / `❌ name`)
    #[serde(default = "default_display_true", alias = "tool_result", alias = "tool_results", alias = "show_tool_results", alias = "show_tool_result")]
    pub show_tool_result: bool,
    /// Show answer header (`💬 answer`)
    #[serde(default = "default_display_true", alias = "answer", alias = "show_answer", alias = "show_answer_label", alias = "answer_label")]
    pub show_answer: bool,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            show_thinking: true,
            show_tool_call: true,
            show_tool_result: true,
            show_answer: true,
        }
    }
}

impl DisplayOptions {
    /// Disable all display options (show plain answer text only)
    pub fn all_disabled() -> Self {
        Self {
            show_thinking: false,
            show_tool_call: false,
            show_tool_result: false,
            show_answer: false,
        }
    }

    /// Enable all display options
    pub fn all_enabled() -> Self {
        Self::default()
    }

    /// Returns true if at least one display section is enabled
    #[allow(dead_code)]
    pub fn has_any_enabled(&self) -> bool {
        self.show_thinking || self.show_tool_call || self.show_tool_result || self.show_answer
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub allowed_paths: Vec<PathBuf>,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub db_path: PathBuf,
    pub wiki_dir: PathBuf,
}

/// Built-in tools assignable to sub-agents via `tools` key
///
/// JSON values are lowercase strings: `"search"`, `"devtools"`, `"datetime"`, `"weather"`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentTool {
    /// Google Search tool via Serper (`google_search`)
    Search,
    /// Web fetch tool (`web_fetch`)
    #[serde(rename = "web_fetch", alias = "webfetch", alias = "fetch")]
    WebFetch,
    /// Workspace DevTools — file read/write + bash
    Devtools,
    /// Filesystem tool
    Filesystem,
    /// Date/time tool (`get_datetime`)
    Datetime,
    /// Weather tool (`get_weather_forecast`)
    Weather,
    /// Media tools via Gemini API (`generate_image`/`generate_tts`/`generate_music`/`generate_video`)
    Media,
}

/// Defines a sub-agent entry under `agents` key
///
/// Root agent delegates specialist tasks via `transfer_to_agent` using `name` as target
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentConfig {
    /// Name passed to `transfer_to_agent` (must be unique among root and other sub-agents)
    pub name: String,
    /// Description shown in transfer enum and root guidance (defaults to name if omitted)
    #[serde(default)]
    pub description: Option<String>,
    /// Sub-agent system instruction
    pub instruction: String,
    /// Built-in tools enabled for this sub-agent (empty = no tools)
    #[serde(default)]
    pub tools: Vec<AgentTool>,
    /// Model override for this sub-agent (inherits root model if omitted)
    #[serde(default)]
    pub model: Option<String>,
    /// When `true`, this sub-agent is skipped during registration
    #[serde(default)]
    pub disabled: bool,
}

impl SubAgentConfig {
    /// Description for transfer enum / guidance — uses name if omitted
    pub fn effective_description(&self) -> &str {
        self.description.as_deref().unwrap_or(&self.name)
    }
}

/// Single entry in `mcpServers` key (mcp.json format)
///
/// Distinguishes stdio from remote by field presence: `command` indicates stdio,
/// `url` indicates remote/streamable-HTTP.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpServerEntry {
    /// stdio server — managed by `McpServerManager` (requires `command`)
    Stdio(McpServerConfig),
    /// remote server — connected directly via streamable HTTP (requires `url`)
    Remote(RemoteMcpServerConfig),
}

/// Configuration for remote MCP Server connected via streamable HTTP
///
/// Co-located with stdio under `mcpServers` key — see [`McpServerEntry`]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteMcpServerConfig {
    /// Endpoint URL of MCP server, e.g. `https://mcp.example.com/v1`
    pub url: String,
    /// When `true`, skips connecting to this server
    #[serde(default)]
    pub disabled: bool,
    /// Custom HTTP headers attached to every request, e.g. `X-Api-Key`
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Bearer token for `Authorization: Bearer ...` (use `headers` for complex auth)
    #[serde(default)]
    pub bearer_token: Option<String>,
}

/// Application Context Compaction configuration
///
/// Uses post-run turn-count interval (`compaction_interval`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamiCompactionConfig {
    /// Enable/disable compaction (when `false`, `EventsCompactionConfig` is omitted)
    #[serde(default = "default_compaction_enabled")]
    pub enabled: bool,
    /// Number of completed conversation invocations before triggering compaction
    #[serde(default = "default_compaction_interval")]
    pub compaction_interval: u32,
    /// Number of events carried over from previous window for summary continuity
    #[serde(default = "default_overlap_size")]
    pub overlap_size: u32,
    /// Model context window size in tokens (documentation reference)
    #[serde(default = "default_context_window")]
    pub context_window: u64,
}

fn default_compaction_enabled() -> bool {
    true
}
fn default_compaction_interval() -> u32 {
    4
}
fn default_overlap_size() -> u32 {
    2
}
fn default_context_window() -> u64 {
    1_048_576
}

impl Default for NamiCompactionConfig {
    fn default() -> Self {
        Self {
            enabled: default_compaction_enabled(),
            compaction_interval: default_compaction_interval(),
            overlap_size: default_overlap_size(),
            context_window: default_context_window(),
        }
    }
}

/// Media generation configuration (image / TTS / music / video via Gemini API)
///
/// All fields have defaults — omitting `media` key in nami.json will load defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// Enable/disable media toolset (when `false`, sub-agents requesting `media` receive no tools)
    #[serde(default = "default_media_enabled")]
    pub enabled: bool,
    /// Image generation model — returns inlineData as image/jpeg
    #[serde(default = "default_media_image_model")]
    pub image_model: String,
    /// Text-to-speech model — returns inlineData as audio/wav
    #[serde(default = "default_media_tts_model")]
    pub tts_model: String,
    /// Voice for TTS (Gemini prebuilt voice)
    #[serde(default = "default_media_tts_voice")]
    pub tts_voice: String,
    /// Video generation model — must support Interactions API
    #[serde(default = "default_media_video_model")]
    pub video_model: String,
    /// Music generation model (Lyria) — returns inlineData as audio/mpeg (~30s clip)
    #[serde(default = "default_media_music_model")]
    pub music_model: String,
    /// Output directory for generated media files
    #[serde(default = "default_media_output_dir")]
    pub output_dir: PathBuf,
    /// Poll interval between video interaction checks (seconds)
    #[serde(default = "default_media_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Maximum execution time per tool invocation (seconds)
    #[serde(default = "default_media_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_media_enabled() -> bool {
    false
}
fn default_media_image_model() -> String {
    "gemini-3.1-flash-image".to_string()
}
fn default_media_tts_model() -> String {
    "gemini-3.8-flash-tts".to_string()
}
fn default_media_tts_voice() -> String {
    "Kore".to_string()
}
fn default_media_video_model() -> String {
    "gemini-omni-1.1-flash".to_string()
}
fn default_media_music_model() -> String {
    "lyria-3-clip-preview".to_string()
}
fn default_media_output_dir() -> PathBuf {
    PathBuf::from("./media")
}
fn default_media_poll_interval_secs() -> u64 {
    5
}
fn default_media_timeout_secs() -> u64 {
    600
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            enabled: default_media_enabled(),
            image_model: default_media_image_model(),
            tts_model: default_media_tts_model(),
            tts_voice: default_media_tts_voice(),
            video_model: default_media_video_model(),
            music_model: default_media_music_model(),
            output_dir: default_media_output_dir(),
            poll_interval_secs: default_media_poll_interval_secs(),
            timeout_secs: default_media_timeout_secs(),
        }
    }
}

impl NamiConfig {
    /// Search for and load default `nami.json`
    ///
    /// Supports loading from project root directory (`./nami.json`)
    pub fn load_default() -> anyhow::Result<Self> {
        if Path::new("nami.json").exists() {
            Self::load_from_file("nami.json")
        } else {
            anyhow::bail!("Config file nami.json not found")
        }
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: NamiConfig = serde_json::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Validates value ranges that the provider would otherwise reject on every request.
    ///
    /// Only `temperature` is checked: `GenerationConfig::validate` accepts 0.0 to 2.0 and
    /// fails per call, so a typo here would cost one failed turn per message instead of one
    /// error at startup.
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(temperature) = self.temperature
            && !(0.0..=2.0).contains(&temperature)
        {
            anyhow::bail!(
                "temperature {temperature} in nami.json is out of range; use a value from 0.0 to 2.0"
            );
        }
        Ok(())
    }

    /// stdio servers (with `command`) from `mcpServers` — passed to `McpServerManager`
    pub fn stdio_servers(&self) -> HashMap<String, McpServerConfig> {
        self.mcp_servers
            .iter()
            .filter_map(|(id, entry)| match entry {
                McpServerEntry::Stdio(cfg) => Some((id.clone(), cfg.clone())),
                McpServerEntry::Remote(_) => None,
            })
            .collect()
    }

    /// Remote servers (with `url`) from `mcpServers` — connected via streamable HTTP
    pub fn remote_servers(&self) -> Vec<(String, RemoteMcpServerConfig)> {
        self.mcp_servers
            .iter()
            .filter_map(|(id, entry)| match entry {
                McpServerEntry::Remote(cfg) => Some((id.clone(), cfg.clone())),
                McpServerEntry::Stdio(_) => None,
            })
            .collect()
    }

    /// Enabled sub-agents (not `disabled`) in config order
    pub fn enabled_agents(&self) -> impl Iterator<Item = &SubAgentConfig> {
        self.agents.iter().filter(|a| !a.disabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_options_defaults() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" }
        }"#;

        let config: NamiConfig = serde_json::from_str(json).unwrap();
        let display = config.cli.display_options();
        assert!(display.show_thinking);
        assert!(display.show_tool_call);
        assert!(display.show_tool_result);
        assert!(display.show_answer);
        assert!(display.has_any_enabled());
    }

    #[test]
    fn test_cli_options_deserialization_full() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" },
            "cli": {
                "user_id": "alice",
                "show_thinking": false,
                "show_tool_call": true,
                "show_tool_result": false,
                "show_answer": false
            }
        }"#;

        let config: NamiConfig = serde_json::from_str(json).unwrap();
        let display = config.cli.display_options();
        assert!(!display.show_thinking);
        assert!(display.show_tool_call);
        assert!(!display.show_tool_result);
        assert!(!display.show_answer);
        assert!(display.has_any_enabled());
    }

    #[test]
    fn test_cli_options_aliases() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" },
            "cli": {
                "thinking": false,
                "tool_calls": false,
                "tool_results": false,
                "answer_label": false
            }
        }"#;

        let config: NamiConfig = serde_json::from_str(json).unwrap();
        let display = config.cli.display_options();
        assert!(!display.show_thinking);
        assert!(!display.show_tool_call);
        assert!(!display.show_tool_result);
        assert!(!display.show_answer);
        assert!(!display.has_any_enabled());
    }

    #[test]
    fn test_cli_options_nested_display() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" },
            "cli": {
                "display": {
                    "show_thinking": true,
                    "show_tool_call": false,
                    "show_tool_result": true
                }
            }
        }"#;

        let config: NamiConfig = serde_json::from_str(json).unwrap();
        let display = config.cli.display_options();
        assert!(display.show_thinking);
        assert!(!display.show_tool_call);
        assert!(display.show_tool_result);
    }
    /// Split into stdio and remote by present fields — mcp.json format
    #[test]
    fn test_mcp_servers_split_stdio_and_remote() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" },
            "mcpServers": {
                "echo-server": { "command": "npx", "args": ["-y", "mcp-echo-server"] },
                "remote-echo": { "url": "https://echo.example.com/mcp", "disabled": true }
            }
        }"#;

        let config: NamiConfig = serde_json::from_str(json).unwrap();

        let stdio = config.stdio_servers();
        assert_eq!(stdio.len(), 1);
        assert_eq!(stdio["echo-server"].command, "npx");

        let remote = config.remote_servers();
        assert_eq!(remote.len(), 1);
        assert_eq!(remote[0].0, "remote-echo");
        assert_eq!(remote[0].1.url, "https://echo.example.com/mcp");
        assert!(remote[0].1.disabled);
    }

    /// Missing `mcpServers` key must not error — returns empty map
    #[test]
    fn test_mcp_servers_absent_is_empty() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" }
        }"#;

        let config: NamiConfig = serde_json::from_str(json).unwrap();
        assert!(config.stdio_servers().is_empty());
        assert!(config.remote_servers().is_empty());
    }

    /// `agents` key parses as `SubAgentConfig` + `AgentTool`, and `disabled` agents are filtered out
    #[test]
    fn test_agents_parse_and_enabled_filter() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" },
            "agents": [
                { "name": "researcher", "instruction": "r", "tools": ["search"] },
                { "name": "coder", "instruction": "c", "tools": ["devtools"], "model": "x" },
                { "name": "off", "instruction": "o", "disabled": true }
            ]
        }"#;

        let config: NamiConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.agents.len(), 3);

        let enabled: Vec<_> = config.enabled_agents().collect();
        assert_eq!(
            enabled.len(),
            2,
            "only the disabled agent must be filtered out"
        );
        assert_eq!(enabled[0].name, "researcher");
        assert_eq!(enabled[0].tools, vec![AgentTool::Search]);
        assert_eq!(enabled[1].name, "coder");
        assert_eq!(enabled[1].model.as_deref(), Some("x"));

        // Missing description falls back to name
        assert_eq!(enabled[0].effective_description(), "researcher");
    }

    /// Missing `agents` key must not error — returns empty list
    #[test]
    fn test_agents_absent_is_empty() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" }
        }"#;

        let config: NamiConfig = serde_json::from_str(json).unwrap();
        assert!(config.agents.is_empty());
        assert_eq!(config.enabled_agents().count(), 0);
    }

    /// Missing `media` key must not error — uses defaults
    #[test]
    fn test_media_absent_uses_defaults() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" }
        }"#;

        let config: NamiConfig = serde_json::from_str(json).unwrap();
        assert!(!config.media.enabled, "media must be opt-in by default");
        assert_eq!(config.media.image_model, "gemini-3.1-flash-image");
        assert_eq!(config.media.tts_model, "gemini-3.8-flash-tts");
        assert_eq!(config.media.tts_voice, "Kore");
        assert_eq!(config.media.video_model, "gemini-omni-1.1-flash");
        assert_eq!(config.media.music_model, "lyria-3-clip-preview");
        assert_eq!(config.media.output_dir, PathBuf::from("./media"));
        assert_eq!(config.media.poll_interval_secs, 5);
        assert_eq!(config.media.timeout_secs, 600);
    }

    /// Partial `media` key merges with defaults
    #[test]
    fn test_media_partial_section_merges_defaults() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" },
            "media": { "enabled": true, "tts_voice": "Puck", "output_dir": "./out" }
        }"#;

        let config: NamiConfig = serde_json::from_str(json).unwrap();
        assert!(config.media.enabled);
        assert_eq!(config.media.tts_voice, "Puck");
        assert_eq!(config.media.output_dir, PathBuf::from("./out"));
        // Unspecified fields remain default
        assert_eq!(config.media.image_model, "gemini-3.1-flash-image");
        assert_eq!(config.media.music_model, "lyria-3-clip-preview");
        assert_eq!(config.media.timeout_secs, 600);
    }

    /// `temperature` out of range 0.0–2.0 fails validate
    #[test]
    fn test_temperature_out_of_range_fails_validate() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s", "temperature": 5.0,
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" }
        }"#;
        let config: NamiConfig = serde_json::from_str(json).unwrap();

        assert!(config.validate().is_err());
    }

    /// Temperature in valid range (0.0 to 2.0) passes validate
    #[test]
    fn test_temperature_in_range_passes_validate() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" }
        }"#;
        let mut config: NamiConfig = serde_json::from_str(json).unwrap();

        assert!(config.validate().is_ok(), "absent temperature must pass");
        for temperature in [0.0, 0.2, 2.0] {
            config.temperature = Some(temperature);
            assert!(
                config.validate().is_ok(),
                "temperature {temperature} must pass"
            );
        }
        config.temperature = Some(-0.1);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_commands_parse_and_default() {
        let json = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" },
            "commands": {
                "review": "Inspect the codebase changes",
                "summarize": "Summarize the project"
            }
        }"#;

        let config: NamiConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.commands.len(), 2);
        assert_eq!(config.commands.get("review").unwrap(), "Inspect the codebase changes");
    }
}
