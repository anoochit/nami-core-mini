//! ชั้น client สำหรับ media generation (image / TTS / music / video) ผ่าน Gemini API
//!
//! image/TTS/music ใช้ wire client ตรง ๆ (`generateContent` + response modalities)
//! เพราะ Llm wrapper ทิ้ง `response_modalities` / `speech_config` ตอนแปลง config
//! (ดู `adk-model` gemini `client.rs`) ส่วน video ปัจจุบันใช้
//! `gemini-omni-1.1-flash` ผ่าน Interactions API (ดู `video.rs`) — ทุกตัว POST
//! ด้วย `reqwest` และ parse response ด้วย typed struct ของ adk-gemini
//!
//! ดูแผนเต็มใน `wiki/multimodal-generation-plan.md` (Phase 0 spike ยืนยัน
//! model IDs ด้วย API call จริงแล้วเมื่อ 2026-10-23)

// API ส่วนนี้ยังไม่มีผู้เรียกจนกว่าจะถึง Phase 2-4 (tools ใน `src/tools/media.rs`)
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

/// base URL ของ Gemini API (AI Studio / v1beta) ที่ทุก tool media ใช้ร่วมกัน
pub const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// POST `models/{model}:generateContent` แล้ว parse response เป็น
/// `GenerationResponse` ของ adk-gemini (typed parse ที่ผ่านการทดสอบใน crate)
///
/// request เป็น JSON ที่ประกอบเอง (ไม่ใช้ typed builder) เพราะ
/// `GenerationConfig` ของ adk-gemini ไม่มี `imageConfig` (aspect ratio) และ
/// `GeminiClient` ไม่เปิด raw execute ภายนอก — ดู `wiki/multimodal-generation-plan.md`
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

/// ตัดข้อความ error ให้สั้น ๆ ก่อนใส่ใน error message (กัน response ยาว ๆ เติม context)
///
/// `pub(crate)` เพราะ video.rs ใช้ร่วมกัน
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

/// host ของ URL เป็นโดเมน Google หรือไม่ (googleapis.com / googleusercontent.com)
///
/// ใช้ตัดสินใจว่าจะแนบ `x-goog-api-key` ตอนดาวน์โหลดไฟล์จาก uri ชี้ไปที่
/// googleusercontent.com แต่จะไม่ส่งคีย์ออกไป host อื่นเด็ดขาด
/// (lookalike เช่น `googleapis.com.download` ถูกปฏิเสธ)
pub(crate) fn host_is_google(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .is_some_and(|h| h.ends_with("googleapis.com") || h.ends_with("googleusercontent.com"))
}

/// อ่าน `GOOGLE_API_KEY` จาก environment (ต้องตั้งผ่าน .env ก่อนรัน)
///
/// ทุกเครื่องมือ media ใช้คีย์เดียวกับ chat model — ไม่มีคีย์ = ไม่มี generation
pub fn require_api_key() -> anyhow::Result<String> {
    validate_api_key(std::env::var("GOOGLE_API_KEY"))
}

/// ตรวจค่า environment value ที่อ่านมา (แยกเป็น function เล็กเพื่อให้ทดสอบ
/// ได้โดยไม่ต้องแตะ env จริง)
fn validate_api_key(value: Result<String, std::env::VarError>) -> anyhow::Result<String> {
    match value {
        Ok(key) if !key.is_empty() && key != "your_gemini_api_key_here" => Ok(key),
        _ => bail!("GOOGLE_API_KEY not set — media generation requires a Google API key in .env"),
    }
}

/// base directory ของ media ทั้งหมด (ตาม `media.output_dir` ใน nami.json)
pub fn output_base(cfg: &MediaConfig) -> PathBuf {
    PathBuf::from(&cfg.output_dir)
}

/// สร้าง run directory ใหม่ `base/run-<yyyymmdd-hhmmss>-<8hex>` สำหรับเก็บ
/// ไฟล์ media + `manifest.json` ของการเรียก tool หนึ่งครั้ง
///
/// สร้างทับไม่ได้ — ถ้าชนกัน (uuid ซ้ำภายในวินาทีเดียวกัน) จะ error
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

    /// key ปกติ = ผ่าน, ว่าง/placeholder/ไม่ตั้ง = error พร้อมข้อความบอกวิธีแก้
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

    /// สร้าง run dir ซ้ำสองครั้งต้องได้คนละ directory ชื่อขึ้นต้นด้วย `run-`
    /// และอยู่ใต้ base ที่กำหนด
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

    /// `output_base` ใช้ `media.output_dir` จากคอนฟิกตามตรง
    #[test]
    fn test_output_base_follows_config() {
        let cfg = MediaConfig {
            output_dir: PathBuf::from("./out"),
            ..Default::default()
        };
        assert_eq!(output_base(&cfg), PathBuf::from("./out"));
    }

    /// error text ถูกตัดที่ 300 อักขระ (นับ char boundary ไม่ใช่ byte) และเติม …
    #[test]
    fn test_truncate_for_error() {
        assert_eq!(truncate_for_error("short"), "short");
        let long = "x".repeat(400);
        let cut = truncate_for_error(&long);
        assert_eq!(cut.chars().count(), 301); // 300 + …
        assert!(cut.ends_with('…'));
        // อักขระหลาย byte (ไทย/อีโมจิ) ต้องไม่ panic ตอนตัด
        let thai = "ก".repeat(200); // 600 bytes
        assert!(truncate_for_error(&thai).chars().count() <= 301);
    }

    /// ไม่ส่ง API key ไป host ที่ไม่ใช่ Google — โดยเฉพาะ lookalike domain
    /// (เช่น `googleapis.com.download` ต้องถูกปฏิเสธ ไม่ใช่ match)
    #[test]
    fn test_host_is_google() {
        assert!(host_is_google(
            "https://generativelanguage.googleapis.com/v1beta/operations/x"
        ));
        assert!(host_is_google(
            "https://storage.googleapis.com/veo-bucket/video.mp4"
        ));
        assert!(!host_is_google("https://evil.example.com/steal"));
        // lookalike domain ห้ามได้คีย์เด็ดขาด
        assert!(!host_is_google(
            "https://generativelanguage.googleapis.com.download/storage/x.mp4"
        ));
        assert!(!host_is_google("not a url"));
    }
}
