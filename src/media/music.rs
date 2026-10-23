//! เรียก Gemini music model (Lyria 3) ผ่าน `generateContent` + `responseModalities: AUDIO`
//!
//! spike จริง 2026-10-23 (ต่างจาก image/TTS ตรงที่ไม่มี config เพิ่มเลย):
//! - `lyria-3-clip-preview` (โมเดล 30 วินาที) คืน part[0] = text `<instrumental>`
//!   (marker) ตามด้วย part[1] = inlineData `audio/mpeg` (MP3 ~30 วินาที)
//! - **ไม่มี parameter ควบคุมความยาว/สไตล์**: ทั้ง `durationSeconds` และ
//!   `musicConfig` ใน `generationConfig` ถูก 400 ปฏิเสธ ("Cannot find field")
//!   — จึงปรับสไตล์ผ่าน prompt ล้วน ๆ
//! - โมเดลเพลงอื่นที่ key นี้เห็น: `lyria-3.5`, `lyria-3-pro-preview`
//!   (แบบ realtime คือ `lyria-realtime-exp`/`bidiGenerateMusic` — ไม่ใช้ที่นี่)
//!
//! ห้ามส่ง base64 เข้า context ของ agent (best practice #2 ของ
//! `wiki/multi-model-generation.md`) — tool layer ต้องเขียนลง disk ทันที

use adk_gemini::{GenerationResponse, Part};
use anyhow::{Context, bail};
use std::time::Duration;

/// ผลลัพธ์ดนตรีหนึ่งแทร็ก (การเรียกหนึ่งครั้งคืน 1 เพลง คลิป ~30 วินาที)
#[derive(Debug)]
pub struct GeneratedMusic {
    /// MIME type ที่ API คืนมาจริง เช่น `audio/mpeg` (ห้ามเดาเอง)
    pub mime_type: String,
    /// ข้อมูลเสียงถอด base64 แล้ว
    pub data: Vec<u8>,
    /// ข้อความ accompanying ที่ model คืนมา (มักเป็น `<instrumental>`)
    pub text: Option<String>,
}

/// ประกอบ request body สำหรับ music generation
///
/// ไม่มี config เพิ่มเติม — spike ยืนยันว่า `generationConfig` รับเฉพาะ
/// `responseModalities` (pure function — ทดสอบได้โดยไม่ต้องยิง API)
pub fn build_request_body(prompt: &str) -> serde_json::Value {
    serde_json::json!({
        "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
        "generationConfig": { "responseModalities": ["AUDIO"] },
    })
}

/// ต่อ style/genre hint เข้าไปใน prompt (โมเดลไม่มี param รับสไตล์โดยตรง)
///
/// pure function — ไม่มี style คืน prompt เดิมทั้งหมด
pub fn compose_music_prompt(prompt: &str, style: Option<&str>) -> String {
    match style {
        Some(s) => format!("{prompt}\nStyle: {s}."),
        None => prompt.to_string(),
    }
}

/// MIME type ของผลลัพธ์ → นามสกุลไฟล์ (ไม่รู้จัก = `bin` กันชนกัน)
pub fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/flac" => "flac",
        "audio/ogg" => "ogg",
        _ => "bin",
    }
}

/// เรียก music model หนึ่งครั้ง แล้วคืนแทร็กแรกที่พบ
///
/// model คืน part ได้หลายแบบ — เก็บ audio inlineData ตัวแรกที่ mime ขึ้นต้นด้วย
/// `audio/` และ text accompanying (ถ้ามี)
pub async fn generate(
    api_key: &str,
    model: &str,
    prompt: &str,
    timeout: Duration,
) -> anyhow::Result<GeneratedMusic> {
    if prompt.trim().is_empty() {
        bail!("prompt is empty — describe the music you want");
    }

    let body = build_request_body(prompt);
    let response: GenerationResponse =
        crate::media::post_generate_content(api_key, model, &body, timeout).await?;

    let Some(candidate) = response.candidates.first() else {
        bail!(
            "model returned no candidates (promptFeedback: {:?})",
            response.prompt_feedback
        );
    };

    let mut track: Option<GeneratedMusic> = None;
    let mut text: Option<String> = None;
    for part in candidate.content.parts.clone().unwrap_or_default() {
        match part {
            Part::InlineData { inline_data } if inline_data.mime_type.starts_with("audio/") => {
                if track.is_some() {
                    continue; // เก็บแทร็กแรกเท่านั้น
                }
                let data = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    &inline_data.data,
                )
                .context("music inlineData is not valid base64")?;
                track = Some(GeneratedMusic {
                    mime_type: inline_data.mime_type,
                    data,
                    text: None,
                });
            }
            Part::Text { text: t, .. } if text.is_none() => text = Some(t),
            _ => {}
        }
    }

    let Some(mut music) = track else {
        bail!(
            "model returned no audio data (text: {text:?}) — check that {} supports AUDIO output",
            model
        );
    };
    music.text = text;
    Ok(music)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// body มี role/parts + responseModalities ["AUDIO"] เท่านั้น (ไม่มี config อื่น)
    #[test]
    fn test_build_request_body() {
        let body = build_request_body("calm piano piece");
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "calm piano piece");
        assert_eq!(
            body["generationConfig"]["responseModalities"],
            serde_json::json!(["AUDIO"])
        );
        assert_eq!(
            body["generationConfig"].as_object().unwrap().len(),
            1,
            "no extra config keys — spike ยืนยันว่า API ปฏิเสธ field อื่น"
        );
    }

    /// style ถูกต่อเป็น prompt hint; ไม่มี style = prompt เดิมเป๊ะ
    #[test]
    fn test_compose_music_prompt() {
        assert_eq!(compose_music_prompt("p", None), "p");
        let out = compose_music_prompt("upbeat track", Some("8-bit chiptune"));
        assert!(out.starts_with("upbeat track"));
        assert!(out.contains("Style: 8-bit chiptune."));
    }

    /// MIME → นามสกุล และ default "bin" สำหรับที่ไม่รู้จัก
    #[test]
    fn test_extension_for_mime() {
        assert_eq!(extension_for_mime("audio/mpeg"), "mp3");
        assert_eq!(extension_for_mime("audio/wav"), "wav");
        assert_eq!(extension_for_mime("audio/flac"), "flac");
        assert_eq!(extension_for_mime("audio/ogg"), "ogg");
        assert_eq!(extension_for_mime("application/octet-stream"), "bin");
    }

    /// prompt ว่าง = error ก่อนยิง API (ไม่ต้องใช้คีย์)
    #[tokio::test]
    async fn test_generate_rejects_empty_prompt() {
        let err = generate("unused-key", "m", "  ", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("prompt is empty"));
    }

    /// round-trip กับ API จริง — เสียเงินเล็กน้อย (คลิป ~30 วินาที)
    /// รันด้วย: `cargo test media::music::tests::test_generate_live -- --ignored`
    #[tokio::test]
    #[ignore = "requires GOOGLE_API_KEY; makes a real (cheap) music API call"]
    async fn test_generate_live() {
        // test binary ไม่ได้รัน main() จึงต้องโหลด .env เอง (เหมือนที่ nami จริงทำ)
        let _ = dotenvy::dotenv();
        let Ok(key) = crate::media::require_api_key() else {
            eprintln!("skipping: GOOGLE_API_KEY not set");
            return;
        };
        let music = generate(
            &key,
            "lyria-3-clip-preview",
            "Upbeat instrumental electronic music with a steady drum beat",
            std::time::Duration::from_secs(120),
        )
        .await
        .expect("live music generation");
        assert!(music.data.len() > 100_000, "track too small");
        assert_eq!(music.mime_type, "audio/mpeg");
        // MP3: ขึ้นต้นด้วย ID3 tag (49 44 33) หรือ frame sync (FF FB/FA/F3/F2)
        let d = &music.data;
        let is_id3 = d[0] == 0x49 && d[1] == 0x44 && d[2] == 0x33;
        let is_sync = d[0] == 0xFF && (d[1] & 0xE0) == 0xE0;
        assert!(
            is_id3 || is_sync,
            "not an MP3: {:02X?}",
            &d[..4.min(d.len())]
        );
    }
}
