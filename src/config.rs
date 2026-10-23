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
    /// การตั้งค่า HTTP Server
    #[serde(default)]
    pub server: ServerOptionsConfig,
    /// การตั้งค่า CLI client
    #[serde(default)]
    pub cli: CliOptionsConfig,
    /// MCP Servers ทั้ง stdio และ remote ในคีย์ `mcpServers` (รูปแบบ mcp.json)
    /// entry ที่มี `command` คือ stdio (จัดการโดย `McpServerManager`)
    /// entry ที่มี `url` คือ remote/streamable-HTTP (เชื่อมต่อโดยตรง)
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: HashMap<String, McpServerEntry>,
    /// การตั้งค่า Context Compaction (post-run, แบบนับจำนวนรอบสนทนา)
    #[serde(default)]
    pub compaction: NamiCompactionConfig,
    /// Sub-Agents แบบ hierarchical — root ส่งงานเฉพาะทางให้ผ่าน `transfer_to_agent`
    /// แต่ละ entry ถูกสร้างเป็น `LlmAgent` แล้วแนบเป็น sub-agent ของ root
    #[serde(default)]
    pub agents: Vec<SubAgentConfig>,
    /// การตั้งค่า media generation (image/TTS/video ผ่าน Gemini API)
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
}

fn default_cli_user_id() -> String {
    "user".to_string()
}

impl Default for CliOptionsConfig {
    fn default() -> Self {
        Self {
            user_id: default_cli_user_id(),
        }
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

/// เครื่องมือ built-in ที่เลือกใส่ sub-agent ได้ผ่านคีย์ `tools`
///
/// ค่าใน JSON เป็นชื่อตัวพิมพ์เล็ก: `"search"`, `"devtools"`, `"datetime"`, `"weather"`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentTool {
    /// เครื่องมือค้นหา Google ผ่าน Serper (`google_search`)
    Search,
    /// Workspace DevTools — อ่าน/เขียนไฟล์ + bash
    Devtools,
    /// พื้นที่จัดเก็บไฟล์/โฟลเดอร์ในระบบ
    Filesystem,
    /// เครื่องมือวันที่/เวลา (`get_datetime`)
    Datetime,
    /// เครื่องมือสภาพอากาศ (`get_weather_forecast`)
    Weather,
    /// ชุดเครื่องมือ media ผ่าน Gemini API (`generate_image`/`generate_tts`/`generate_music`/`generate_video`)
    Media,
}

/// นิยาม sub-agent หนึ่งตัวในคีย์ `agents`
///
/// root จะส่งงานให้ผ่าน `transfer_to_agent` โดยใช้ `name` เป็นเป้าหมาย
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentConfig {
    /// ชื่อที่ใช้เรียกผ่าน `transfer_to_agent` (ต้องไม่ซ้ำกับ root หรือ sub-agent อื่น)
    pub name: String,
    /// คำอธิบาย — แสดงใน transfer enum และใน guidance ของ root (ถ้าไม่ระบุใช้ชื่อแทน)
    #[serde(default)]
    pub description: Option<String>,
    /// system instruction ของ sub-agent
    pub instruction: String,
    /// เครื่องมือ built-in ที่ sub-agent นี้ใช้ได้ (ค่าว่าง = ไม่มีเครื่องมือ)
    #[serde(default)]
    pub tools: Vec<AgentTool>,
    /// model ที่ใช้เฉพาะ sub-agent นี้ (ถ้าไม่ระบุจะใช้ model ของ root)
    #[serde(default)]
    pub model: Option<String>,
    /// เมื่อ `true` จะข้าม sub-agent ตัวนี้ไม่ลงทะเบียน
    #[serde(default)]
    pub disabled: bool,
}

impl SubAgentConfig {
    /// คำอธิบายสำหรับ transfer enum/guidance — ใช้ชื่อแทนเมื่อไม่ได้ระบุ
    pub fn effective_description(&self) -> &str {
        self.description.as_deref().unwrap_or(&self.name)
    }
}

/// หนึ่ง entry ในคีย์ `mcpServers` (รูปแบบ mcp.json)
///
/// แยก stdio ออกจาก remote ตาม field ที่มี: มี `command` คือ stdio,
/// มี `url` คือ remote/streamable-HTTP — ทั้งสองชนิดอยู่ในคีย์เดียวกัน
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpServerEntry {
    /// stdio server — จัดการโดย `McpServerManager` (ต้องมี `command`)
    Stdio(McpServerConfig),
    /// remote server — เชื่อมต่อโดยตรงผ่าน streamable HTTP (ต้องมี `url`)
    Remote(RemoteMcpServerConfig),
}

/// คอนฟิกสำหรับ MCP Server ระยะไกลเชื่อมต่อผ่าน streamable HTTP
///
/// อยู่รวมกับ stdio ในคีย์ `mcpServers` — ดู [`McpServerEntry`]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteMcpServerConfig {
    /// endpoint ของ MCP server เช่น `https://mcp.example.com/v1`
    pub url: String,
    /// เมื่อ `true` จะข้าม server ตัวนี้ไม่พยายามเชื่อมต่อ
    #[serde(default)]
    pub disabled: bool,
    /// header เพิ่มเติมที่แนบไปกับทุกคำขอ เช่น `X-Api-Key`
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// bearer token สั้น ๆ สำหรับ `Authorization: Bearer ...` (ใช้ `headers` สำหรับกรณีซับซ้อน)
    #[serde(default)]
    pub bearer_token: Option<String>,
}

/// คอนฟิก Context Compaction ระดับแอป
///
/// กลไกที่ใช้ได้กับ wrapper ปัจจุบันคือ **post-run แบบนับจำนวนรอบสนทนา** (`compaction_interval`)
/// ไม่ใช่เปอร์เซ็นต์ของ context window — การกระตุ้นแบบ token-budget ("70% ของ context")
/// ต้องสร้าง `Runner` โดยตรงซึ่ง `Launcher`/`ServerConfig` ใน adk-rust 2.3.0 ไม่เปิดให้ตั้งค่า
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamiCompactionConfig {
    /// เปิด/ปิดการ compaction (เมื่อ `false` จะไม่แนบ `EventsCompactionConfig` เลย)
    #[serde(default = "default_compaction_enabled")]
    pub enabled: bool,
    /// จำนวนรอบสนทนา (invocations) ที่เสร็จสมบูรณ์ก่อนกระตุ้น compaction
    #[serde(default = "default_compaction_interval")]
    pub compaction_interval: u32,
    /// จำนวน event จากหน้าต่างก่อนหน้าที่ส่งต่อไปยังการสรุปถัดไปเพื่อความต่อเนื่อง
    #[serde(default = "default_overlap_size")]
    pub overlap_size: u32,
    /// context window ของโมเดล (tokens) — ใช้อ้างอิง/окументในอนาคตสำหรับการกระตุ้นแบบ token-budget
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

/// การตั้งค่า media generation (image / TTS / music / video ผ่าน Gemini API)
///
/// ค่าทุก field มี default — ไม่ใส่คีย์ `media` ใน nami.json ก็โหลดได้
/// โมเดล/เสียง/ID โมเดลวิดีโอ ระบุในคอนฟิกเท่านั้น (ห้าม hardcode ในโค้ด) เพราะ
/// model IDs ของ Google เปลี่ยนบ่อย — ค่า default ด้านล่างผ่านการ spike
/// ด้วย API call จริงเมื่อ 2026-10-23 (ดู `wiki/multimodal-generation-plan.md`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// เปิด/ปิดชุดเครื่องมือ media (เมื่อ `false` sub-agent ที่เลือก `media`
    /// จะไม่ได้เครื่องมือใด ๆ และได้รับ warning ตอน start)
    #[serde(default = "default_media_enabled")]
    pub enabled: bool,
    /// โมเดลสร้างภาพ — คืน inlineData เป็น image/jpeg
    #[serde(default = "default_media_image_model")]
    pub image_model: String,
    /// โมเดล text-to-speech — คืน inlineData เป็น audio/wav (PCM ห่อด้วย WAV header แล้ว)
    #[serde(default = "default_media_tts_model")]
    pub tts_model: String,
    /// voice ที่ใช้กับ TTS (prebuilt voice ของ Gemini)
    #[serde(default = "default_media_tts_voice")]
    pub tts_voice: String,
    /// โมเดลสร้างวิดีโอ — ต้องเป็นโมเดลที่รับ Interactions API (เช่น `gemini-omni-1.1-flash`)
    /// ส่วน Veo ถูกถอนออกแล้ว (ยิงจริงได้ 404 — ดู `wiki/multimodal-generation-plan.md`)
    #[serde(default = "default_media_video_model")]
    pub video_model: String,
    /// โมเดลสร้างดนตรี (Lyria) — คืน inlineData เป็น audio/mpeg (คลิป ~30 วินาที คงที่
    /// ไม่มี parameter ควบคุมความยาว — spike 2026-10-23 ยืนยัน)
    #[serde(default = "default_media_music_model")]
    pub music_model: String,
    /// ไดเรกทอรีเก็บผลลัพธ์ — สร้าง run dir ย่อย `run-<ts>-<id>` ต่อการเรียก tool หนึ่งครั้ง
    #[serde(default = "default_media_output_dir")]
    pub output_dir: PathBuf,
    /// ระยะเวลารอระหว่าง poll interaction ของ video model (วินาที)
    #[serde(default = "default_media_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// เวลาสูงสุดต่อการเรียก tool หนึ่งครั้ง (วินาที) — กัน video call ค้างค่า
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
    /// ค้นหาและโหลดไฟล์ `nami.json` จากตำแหน่งที่เป็นไปได้
    ///
    /// รองรับทั้งการรันจากไดเรกทอรีโปรเจกต์เอง (`./nami.json`)
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

    /// stdio servers (มี `command`) จากคีย์ `mcpServers` — ส่งให้ `McpServerManager`
    pub fn stdio_servers(&self) -> HashMap<String, McpServerConfig> {
        self.mcp_servers
            .iter()
            .filter_map(|(id, entry)| match entry {
                McpServerEntry::Stdio(cfg) => Some((id.clone(), cfg.clone())),
                McpServerEntry::Remote(_) => None,
            })
            .collect()
    }

    /// remote servers (มี `url`) จากคีย์ `mcpServers` — เชื่อมต่อโดยตรงผ่าน streamable HTTP
    pub fn remote_servers(&self) -> Vec<(String, RemoteMcpServerConfig)> {
        self.mcp_servers
            .iter()
            .filter_map(|(id, entry)| match entry {
                McpServerEntry::Remote(cfg) => Some((id.clone(), cfg.clone())),
                McpServerEntry::Stdio(_) => None,
            })
            .collect()
    }

    /// sub-agents ที่ยังไม่ถูก `disabled` — เรียงตามลำดับในคอนฟิก
    pub fn enabled_agents(&self) -> impl Iterator<Item = &SubAgentConfig> {
        self.agents.iter().filter(|a| !a.disabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// stdio (`command`) และ remote (`url`) อยู่ร่วมกันในคีย์ `mcpServers` เดียว
    /// แล้วแยกกันตาม field ที่มี — รูปแบบ mcp.json
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

    /// ไม่มีคีย์ `mcpServers` ต้องไม่ error — คืน map ว่าง
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

    /// คีย์ `agents` parse เป็น `SubAgentConfig` + `AgentTool` และ `disabled` ถูกกรองออก
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

        // ไม่ระบุ description ต้อง fallback ไปใช้ชื่อ
        assert_eq!(enabled[0].effective_description(), "researcher");
    }

    /// ไม่มีคีย์ `agents` ต้องไม่ error — คืนลิสต์ว่าง
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

    /// ไม่มีคีย์ `media` ต้องไม่ error — ใช้ default (ปิดอยู่, model IDs ที่ spike ไว้)
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

    /// คีย์ `media` บางส่วนต้อง merge กับ default (field ที่ไม่ระบุยังได้ค่า default)
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
        // field ที่ไม่ได้ระบุยังเป็น default
        assert_eq!(config.media.image_model, "gemini-3.1-flash-image");
        assert_eq!(config.media.music_model, "lyria-3-clip-preview");
        assert_eq!(config.media.timeout_secs, 600);
    }

    /// `temperature` นอกช่วง 0.0–2.0 ต้อง fail ตอนโหลด ไม่ใช่ตอนยิง request ทุกครั้ง
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

    /// ค่าในช่วงที่ถูกต้อง (รวมขอบ 0.0 กับ 2.0) และการไม่ระบุค่า ต้องผ่าน validate
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
}
