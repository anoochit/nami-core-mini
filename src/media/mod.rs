//! Client layer for media generation (image / TTS / music / video) via Gemini API
//!
//! image/TTS/music use direct wire client (`generateContent` + response modalities)
//! because Llm wrapper drops `response_modalities` / `speech_config` when converting config
//! (see `adk-model` gemini `client.rs`). Video currently uses `gemini-omni-1.1-flash` via
//! Interactions API (see `video.rs`) — all POST via `reqwest` and parse responses using typed
//! structs from adk-gemini.
//!
//! See full plan in `wiki/multimodal-generation-plan.md`

#![allow(dead_code)]

pub mod assemble;
pub mod image;
pub mod manifest;
pub mod music;
pub mod tts;
pub mod video;

use crate::config::MediaConfig;
use anyhow::{Context, bail};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Base URL for Gemini API (AI Studio / v1beta) shared across all media tools
pub const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// POST to `models/{model}:generateContent` and parse response as `GenerationResponse`
/// from adk-gemini
///
/// Uses custom JSON body construction because `GenerationConfig` in adk-gemini lacks
/// `imageConfig` (aspect ratio) and `GeminiClient` does not expose raw execution externally.
pub(crate) async fn post_generate_content(
    api_key: &str,
    model: &str,
    body: &serde_json::Value,
    timeout: Duration,
) -> anyhow::Result<adk_gemini::GenerationResponse> {
    let url = format!("{GEMINI_BASE_URL}/models/{model}:generateContent");
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .context("cannot build HTTP client")?;
    let resp = client
        .post(&url)
        .header("x-goog-api-key", api_key)
        .json(body)
        .send()
        .await
        .with_context(|| format!("network error calling {model}"))?;
    let status = resp.status();
    let text = resp.text().await.context("cannot read response body")?;
    if !status.is_success() {
        bail!(
            "Gemini API returned {status}: {}",
            truncate_for_error(&text)
        );
    }
    serde_json::from_str(&text).with_context(|| {
        format!(
            "cannot parse generateContent response: {}",
            truncate_for_error(&text)
        )
    })
}

/// Truncate error response body to prevent huge contexts in error messages
///
/// `pub(crate)` because video.rs shares this function
pub(crate) fn truncate_for_error(text: &str) -> String {
    const MAX: usize = 300;
    if text.len() <= MAX {
        return text.to_string();
    }
    let mut end = MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// Whether URL host is a Google domain (googleapis.com / googleusercontent.com)
///
/// Used to decide whether to attach `x-goog-api-key` when downloading files from URIs.
/// Will never send keys to external hosts (lookalike domains such as `googleapis.com.download` are rejected).
pub(crate) fn host_is_google(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .is_some_and(|h| h.ends_with("googleapis.com") || h.ends_with("googleusercontent.com"))
}

/// Read `GOOGLE_API_KEY` from environment (.env must be loaded prior)
///
/// All media tools share the same API key as chat models — missing key fails generation
pub fn require_api_key() -> anyhow::Result<String> {
    validate_api_key(std::env::var("GOOGLE_API_KEY"))
}

/// Validate environment value (separated as helper for testing without touching env)
fn validate_api_key(value: Result<String, std::env::VarError>) -> anyhow::Result<String> {
    match value {
        Ok(key) if !key.is_empty() && key != "your_gemini_api_key_here" => Ok(key),
        _ => bail!("GOOGLE_API_KEY not set — media generation requires a Google API key in .env"),
    }
}

/// Base directory for all generated media (per `media.output_dir` in nami.json)
pub fn output_base(cfg: &MediaConfig) -> PathBuf {
    PathBuf::from(&cfg.output_dir)
}

/// Create a new run directory `base/run-<yyyymmdd-hhmmss>-<8hex>` to hold
/// generated media files + `manifest.json` for a single tool invocation
pub fn create_run_dir(base: &Path) -> anyhow::Result<PathBuf> {
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let uuid_full = uuid::Uuid::new_v4().simple().to_string();
    let dir = base.join(format!("run-{ts}-{}", &uuid_full[..8]));
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("cannot create media run dir {}", dir.display()))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Valid key = Ok, empty/placeholder/missing = Err with actionable context
    #[test]
    fn test_validate_api_key() {
        assert!(validate_api_key(Ok("real-key-123".to_string())).is_ok());
        assert!(validate_api_key(Ok(String::new())).is_err());
        assert!(validate_api_key(Ok("your_gemini_api_key_here".to_string())).is_err());
        assert!(
            validate_api_key(Err(std::env::VarError::NotPresent)).is_err(),
            "missing env var must be an error"
        );
    }

    /// Creating run dir twice yields two distinct directories starting with `run-` under base
    #[test]
    fn test_create_run_dir_unique_and_under_base() {
        let base = std::env::temp_dir().join(format!("nami-media-test-{}", uuid::Uuid::new_v4()));
        let d1 = create_run_dir(&base).unwrap();
        let d2 = create_run_dir(&base).unwrap();

        assert!(d1.is_dir(), "run dir must exist on disk");
        assert_ne!(d1, d2, "run dirs must be unique");
        assert!(d1.starts_with(&base) && d2.starts_with(&base));
        let name = d1.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            // "run-" (4) + "yyyymmdd-hhmmss" (15) + "-" (1) + 8 hex = 28
            name.starts_with("run-") && name.len() == 4 + 15 + 1 + 8,
            "unexpected run dir name: {name}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// `output_base` follows `media.output_dir` from config directly
    #[test]
    fn test_output_base_follows_config() {
        let cfg = MediaConfig {
            output_dir: PathBuf::from("./out"),
            ..Default::default()
        };
        assert_eq!(output_base(&cfg), PathBuf::from("./out"));
    }

    /// Error text is truncated at 300 chars (respecting character boundary) with appended …
    #[test]
    fn test_truncate_for_error() {
        assert_eq!(truncate_for_error("short"), "short");
        let long = "x".repeat(400);
        let cut = truncate_for_error(&long);
        assert_eq!(cut.chars().count(), 301); // 300 + …
        assert!(cut.ends_with('…'));
        // Multi-byte chars (Thai/emoji) must not panic on slice boundary
        let thai = "ก".repeat(200); // 600 bytes
        assert!(truncate_for_error(&thai).chars().count() <= 301);
    }

    /// Do not send API key to non-Google hosts — especially lookalike domains
    /// (e.g. `googleapis.com.download` must be rejected, not matched)
    #[test]
    fn test_host_is_google() {
        assert!(host_is_google(
            "https://generativelanguage.googleapis.com/v1beta/operations/x"
        ));
        assert!(host_is_google(
            "https://storage.googleapis.com/veo-bucket/video.mp4"
        ));
        assert!(!host_is_google("https://evil.example.com/steal"));
        // lookalike domains must never receive keys
        assert!(!host_is_google(
            "https://generativelanguage.googleapis.com.download/storage/x.mp4"
        ));
        assert!(!host_is_google("not a url"));
    }
}