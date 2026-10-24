//! Call Gemini music model (Lyria 3) via `generateContent` + `responseModalities: AUDIO`
//!
//! Verified via spike on 2026-10-23 (unlike image/TTS, there are no extra config options):
//! - `lyria-3-clip-preview` (30s model) returns part[0] = text `<instrumental>`
//!   (marker) followed by part[1] = inlineData `audio/mpeg` (MP3 ~30 seconds)
//! - **No parameters controlling duration/style**: Both `durationSeconds` and `musicConfig`
//!   in `generationConfig` are rejected with 400 ("Cannot find field")
//!   — style is adjusted purely through the prompt
//! - Other music models accessible with this key: `lyria-3.5`, `lyria-3-pro-preview`
//!   (realtime variants are `lyria-realtime-exp`/`bidiGenerateMusic` — not used here)
//!
//! Never send base64 into agent context (best practice #2 in
//! `wiki/multi-model-generation.md`) — tool layer must write to disk immediately.

use adk_gemini::{GenerationResponse, Part};
use anyhow::{Context, bail};
use std::time::Duration;

/// Result of a single music track generation (1 call returns 1 track, ~30s clip)
#[derive(Debug)]
pub struct GeneratedMusic {
    /// Actual MIME type returned by the API, e.g. `audio/mpeg` (do not guess)
    pub mime_type: String,
    /// Audio binary data decoded from base64
    pub data: Vec<u8>,
    /// Accompanying text returned by model (usually `<instrumental>`)
    pub text: Option<String>,
}

/// Build request body for music generation
///
/// No extra config — spike confirmed `generationConfig` only accepts `responseModalities`
pub fn build_request_body(prompt: &str) -> serde_json::Value {
    serde_json::json!({
        "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
        "generationConfig": { "responseModalities": ["AUDIO"] },
    })
}

/// Append style/genre hint to prompt (model has no direct style parameter)
///
/// Pure function — if no style, returns original prompt unchanged
pub fn compose_music_prompt(prompt: &str, style: Option<&str>) -> String {
    match style {
        Some(s) => format!("{prompt}\nStyle: {s}."),
        None => prompt.to_string(),
    }
}

/// Map result MIME type to file extension (fallback to `bin` for unknown)
pub fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/flac" => "flac",
        "audio/ogg" => "ogg",
        _ => "bin",
    }
}

/// Call music model once and return the first track found
///
/// Model returns parts in various forms — captures the first inlineData with MIME starting
/// with `audio/` and accompanying text (if any).
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
                    continue; // Keep first track only
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

    /// Request body contains role/parts + responseModalities ["AUDIO"] only (no other config)
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
            "no extra config keys — spike confirmed API rejects other fields"
        );
    }

    /// Style appended as prompt hint; no style = exact original prompt
    #[test]
    fn test_compose_music_prompt() {
        assert_eq!(compose_music_prompt("p", None), "p");
        let out = compose_music_prompt("upbeat track", Some("8-bit chiptune"));
        assert!(out.starts_with("upbeat track"));
        assert!(out.contains("Style: 8-bit chiptune."));
    }

    /// MIME → extension and default "bin" for unknown types
    #[test]
    fn test_extension_for_mime() {
        assert_eq!(extension_for_mime("audio/mpeg"), "mp3");
        assert_eq!(extension_for_mime("audio/wav"), "wav");
        assert_eq!(extension_for_mime("audio/flac"), "flac");
        assert_eq!(extension_for_mime("audio/ogg"), "ogg");
        assert_eq!(extension_for_mime("application/octet-stream"), "bin");
    }

    /// Empty prompt = error before API call
    #[tokio::test]
    async fn test_generate_rejects_empty_prompt() {
        let err = generate("unused-key", "m", "  ", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("prompt is empty"));
    }

    /// Live round-trip with real API — requires GOOGLE_API_KEY (~30s clip)
    /// Run with: `cargo test media::music::tests::test_generate_live -- --ignored`
    #[tokio::test]
    #[ignore = "requires GOOGLE_API_KEY; makes a real (cheap) music API call"]
    async fn test_generate_live() {
        // Test binary doesn't run main(), load .env manually
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
        // MP3 header starts with ID3 tag (49 44 33) or frame sync (FF FB/FA/F3/F2)
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