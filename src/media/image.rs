//! Calls Gemini image model via `generateContent` + `responseModalities: IMAGE`
//!
//! Images are decoded from base64 and returned as raw bytes immediately — never send base64 into
//! agent context (best practice #2 in `wiki/multi-model-generation.md`). Caller (tool layer)
//! must write to disk promptly.

use adk_gemini::{GenerationResponse, Part};
use anyhow::{Context, bail};
use std::time::Duration;

/// Result of an image generation call (typically returns 1 image per request)
#[derive(Debug)]
pub struct GeneratedImage {
    /// Actual MIME type returned by the API, e.g. `image/jpeg` (do not guess)
    pub mime_type: String,
    /// Image binary data decoded from base64
    pub data: Vec<u8>,
    /// Accompanying text returned by the model alongside the image (if any)
    pub text: Option<String>,
}

/// Supported aspect ratios for Gemini image model (prevents typos — errors out if invalid)
pub const SUPPORTED_ASPECT_RATIOS: &[&str] = &[
    "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9",
];

/// Build JSON request body for image generation
///
/// Separated as a pure function for testing without API calls — structure verified via spike.
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

/// Map result MIME type to file extension (fallback to `bin` for unknown)
pub fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        _ => "bin",
    }
}

/// Call the image model once and return all candidate images (typically 1 image)
///
/// `aspect_ratio` must be listed in [`SUPPORTED_ASPECT_RATIOS`] or `None`
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
    // Attach accompanying text to the first image
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

    /// Request body must contain role/parts and responseModalities ["TEXT","IMAGE"]
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

    /// Specifying aspect_ratio must appear as imageConfig.aspectRatio (camelCase wire format)
    #[test]
    fn test_build_request_body_with_aspect_ratio() {
        let body = build_request_body("p", Some("16:9"));
        assert_eq!(
            body["generationConfig"]["imageConfig"]["aspectRatio"],
            "16:9"
        );
    }

    /// MIME → extension and default "bin" for unknown types
    #[test]
    fn test_extension_for_mime() {
        assert_eq!(extension_for_mime("image/jpeg"), "jpg");
        assert_eq!(extension_for_mime("image/png"), "png");
        assert_eq!(extension_for_mime("image/webp"), "webp");
        assert_eq!(extension_for_mime("image/gif"), "bin");
    }

    /// Aspect ratios outside whitelist must be rejected before API call
    #[tokio::test]
    async fn test_generate_rejects_bad_aspect_ratio() {
        let err = generate("unused-key", "m", "p", Some("7:3"), Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unsupported aspect_ratio"));
    }

    /// Live round-trip with real API — requires GOOGLE_API_KEY
    /// Run with: `cargo test media::image::tests::test_generate_live -- --ignored`
    #[tokio::test]
    #[ignore = "requires GOOGLE_API_KEY and makes a real (cheap) API call"]
    async fn test_generate_live() {
        // Test binary doesn't run main(), load .env manually
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
        // magic bytes: JPEG (FF D8 FF) or PNG (89 50 4E 47)
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