//! เรียก Gemini image model ผ่าน `generateContent` + `responseModalities: IMAGE`
//!
//! รูปถูกถอด base64 แล้วคืนเป็น bytes ทันที — ห้ามส่ง base64 เข้า context ของ
//! agent (best practice #2 ของ `wiki/multi-model-generation.md`) ผู้เรียก
//! (tool layer) ต้องรีบเขียนลง disk เอง

use adk_gemini::{GenerationResponse, Part};
use anyhow::{Context, bail};
use std::time::Duration;

/// ผลลัพธ์รูปภาพหนึ่งไฟล์จาก image model (ปกติการเรียกหนึ่งครั้งคืน 1 รูป)
#[derive(Debug)]
pub struct GeneratedImage {
    /// MIME type ที่ API คืนมาจริง เช่น `image/jpeg` (ห้ามเดาเอง)
    pub mime_type: String,
    /// ข้อมูลรูปถอด base64 แล้ว
    pub data: Vec<u8>,
    /// ข้อความ accompanying ที่ model คืนมาพร้อมรูป (ถ้ามี)
    pub text: Option<String>,
}

/// aspect ratios ที่ Gemini image รองรับ (กัน typo — คืน error ถ้าไม่อยู่ในรายการ)
pub const SUPPORTED_ASPECT_RATIOS: &[&str] = &[
    "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9",
];

/// ประกอบ request body สำหรับ image generation
///
/// แยกเป็น pure function เพื่อให้ทดสอบได้โดยไม่ต้องยิง API — รูปแบบผ่านการ
/// spike จริงแล้วเมื่อ 2026-10-23 (ดู wiki/multimodal-generation-plan.md §6)
pub fn build_request_body(prompt: &str, aspect_ratio: Option<&str>) -> serde_json::Value {
    let mut generation_config = serde_json::json!({ "responseModalities": ["TEXT", "IMAGE"] });
    if let Some(ar) = aspect_ratio {
        generation_config["imageConfig"] = serde_json::json!({ "aspectRatio": ar });
    }
    serde_json::json!({
        "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
        "generationConfig": generation_config,
    })
}

/// MIME type ของผลลัพธ์ → นามสกุลไฟล์ (ไม่รู้จัก = `bin` กันชนกัน)
pub fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        _ => "bin",
    }
}

/// เรียก image model หนึ่งครั้ง แล้วคืนรูปทุก candidate (ปกติ 1 รูป)
///
/// `aspect_ratio` ต้องอยู่ใน [`SUPPORTED_ASPECT_RATIOS`] หรือเป็น `None`
pub async fn generate(
    api_key: &str,
    model: &str,
    prompt: &str,
    aspect_ratio: Option<&str>,
    timeout: Duration,
) -> anyhow::Result<Vec<GeneratedImage>> {
    if let Some(ar) = aspect_ratio
        && !SUPPORTED_ASPECT_RATIOS.contains(&ar)
    {
        bail!(
            "unsupported aspect_ratio {ar:?} — supported: {}",
            SUPPORTED_ASPECT_RATIOS.join(", ")
        );
    }

    let body = build_request_body(prompt, aspect_ratio);
    let response: GenerationResponse =
        crate::media::post_generate_content(api_key, model, &body, timeout).await?;

    let Some(candidate) = response.candidates.first() else {
        bail!(
            "model returned no candidates (promptFeedback: {:?})",
            response.prompt_feedback
        );
    };

    let mut images: Vec<GeneratedImage> = Vec::new();
    let mut text: Option<String> = None;
    for part in candidate.content.parts.clone().unwrap_or_default() {
        match part {
            Part::InlineData { inline_data } => {
                let data = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    &inline_data.data,
                )
                .context("image inlineData is not valid base64")?;
                images.push(GeneratedImage {
                    mime_type: inline_data.mime_type,
                    data,
                    text: None,
                });
            }
            Part::Text { text: t, .. } if text.is_none() => text = Some(t),
            _ => {}
        }
    }

    if images.is_empty() {
        bail!(
            "model returned no image data (text: {text:?}) — check that {} supports image output",
            model
        );
    }
    // ข้อความ accompanying ติดไปกับไฟล์แรก
    if let Some(t) = text
        && let Some(first) = images.first_mut()
    {
        first.text = Some(t);
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// body ต้องมี role/parts และ responseModalities ["TEXT","IMAGE"]
    #[test]
    fn test_build_request_body_minimal() {
        let body = build_request_body("a wave", None);
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "a wave");
        assert_eq!(
            body["generationConfig"]["responseModalities"],
            serde_json::json!(["TEXT", "IMAGE"])
        );
        assert!(
            body["generationConfig"].get("imageConfig").is_none(),
            "no imageConfig when aspect_ratio is None"
        );
    }

    /// ระบุ aspect_ratio ต้องโผล่เป็น imageConfig.aspectRatio (camelCase ตาม wire format)
    #[test]
    fn test_build_request_body_with_aspect_ratio() {
        let body = build_request_body("p", Some("16:9"));
        assert_eq!(
            body["generationConfig"]["imageConfig"]["aspectRatio"],
            "16:9"
        );
    }

    /// MIME → นามสกุล และ default "bin" สำหรับที่ไม่รู้จัก
    #[test]
    fn test_extension_for_mime() {
        assert_eq!(extension_for_mime("image/jpeg"), "jpg");
        assert_eq!(extension_for_mime("image/png"), "png");
        assert_eq!(extension_for_mime("image/webp"), "webp");
        assert_eq!(extension_for_mime("image/gif"), "bin");
    }

    /// aspect ratio นอก whitelist ต้องถูกปฏิเสธก่อนยิง API (ไม่ต้องใช้คีย์)
    #[tokio::test]
    async fn test_generate_rejects_bad_aspect_ratio() {
        let err = generate("unused-key", "m", "p", Some("7:3"), Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unsupported aspect_ratio"));
    }

    /// round-trip กับ API จริง — ต้องมี GOOGLE_API_KEY และเสียเงินเล็กน้อย
    /// รันด้วย: `cargo test media::image::tests::test_generate_live -- --ignored`
    #[tokio::test]
    #[ignore = "requires GOOGLE_API_KEY and makes a real (cheap) API call"]
    async fn test_generate_live() {
        // test binary ไม่ได้รัน main() จึงต้องโหลด .env เอง (เหมือนที่ nami จริงทำ)
        let _ = dotenvy::dotenv();
        let Ok(key) = crate::media::require_api_key() else {
            eprintln!("skipping: GOOGLE_API_KEY not set");
            return;
        };
        let images = generate(
            &key,
            "gemini-3.1-flash-image",
            "A minimalist single-line drawing of an ocean wave, white background",
            Some("1:1"),
            std::time::Duration::from_secs(120),
        )
        .await
        .expect("live image generation");
        assert!(!images.is_empty());
        assert!(images[0].data.len() > 1_000, "image bytes too small");
        // magic bytes: JPEG (FF D8 FF) หรือ PNG (89 50 4E 47)
        let d = &images[0].data;
        let is_jpeg = d[0] == 0xFF && d[1] == 0xD8;
        let is_png = d[0] == 0x89 && d[1] == 0x50;
        assert!(
            is_jpeg || is_png,
            "unexpected image format: {:02X?}",
            &d[..4.min(d.len())]
        );
    }
}
