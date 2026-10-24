//! Call Gemini TTS model via `generateContent` + `responseModalities: AUDIO`
//!
//! Spike on 2026-10-23 confirmed `gemini-3.8-flash-tts` returns `audio/wav` with complete headers —
//! saved directly as `.wav` file without manual wrapping.
//! Audio is returned as raw binary — never send base64 into agent context (best practice #2 in
//! `wiki/multi-model-generation.md`)

use adk_gemini::{GenerationResponse, Part};
use anyhow::{Context, bail};
use std::time::Duration;

/// Result of a single speech generation call (typically returns 1 file)
#[derive(Debug)]
pub struct GeneratedSpeech {
    /// Actual MIME type returned by the API, e.g. `audio/wav`
    pub mime_type: String,
    /// Audio binary data (decoded from base64)
    pub data: Vec<u8>,
}

/// Build request body for TTS
///
/// `emotion` is a delivery hint prepended to text (Gemini TTS lacks direct style parameters —
/// delivery guidance is expressed naturally in the prompt)
///
/// Pure function testable without API calls — structure verified via spike on 2026-10-23
pub fn build_request_body(text: &str, voice: &str) -> serde_json::Value {
    serde_json::json!({
        "contents": [{ "role": "user", "parts": [{ "text": text }] }],
        "generationConfig": {
            "responseModalities": ["AUDIO"],
            "speechConfig": {
                "voiceConfig": { "prebuiltVoiceConfig": { "voiceName": voice } }
            }
        },
    })
}

/// Combine spoken text with delivery hint (emotion/tone)
///
/// Hint prepended to actual text, e.g.:
/// `Deliver the following text in a cheerful tone.\n\nHello from Nami.`
pub fn compose_speech_text(text: &str, emotion: Option<&str>) -> String {
    match emotion {
        Some(e) if !e.trim().is_empty() => format!(
            "Deliver the following text in a {} tone.\n\n{}",
            e.trim(),
            text
        ),
        _ => text.to_string(),
    }
}

/// Map result MIME type to file extension
///
/// Model currently returns `audio/wav` — other types (raw PCM/mp3) mapped per API result
pub fn extension_for_mime(mime: &str) -> &'static str {
    let lower = mime.to_ascii_lowercase();
    if lower.starts_with("audio/wav") || lower.starts_with("audio/x-wav") {
        "wav"
    } else if lower.starts_with("audio/mpeg") {
        "mp3"
    } else {
        "raw"
    }
}

/// Estimate audio duration (seconds) from standard WAV header (RIFF/PCM)
///
/// Reads `byteRate` (offset 28, u32 LE) and divides data size — returns `None` if
/// not a standard readable WAV (e.g. raw PCM)
pub fn estimate_wav_duration_secs(data: &[u8]) -> Option<f64> {
    if data.len() < 44 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }
    let byte_rate = u32::from_le_bytes([data[28], data[29], data[30], data[31]]);
    if byte_rate == 0 {
        return None;
    }
    // Standard PCM header uses 44 bytes (RIFF 12 + fmt 24 + data 8)
    let audio_bytes = data.len().saturating_sub(44) as f64;
    Some(audio_bytes / byte_rate as f64)
}

/// Call TTS model once and return speech files for all candidates (typically 1 file)
pub async fn generate(
    api_key: &str,
    model: &str,
    text: &str,
    voice: &str,
    timeout: Duration,
) -> anyhow::Result<Vec<GeneratedSpeech>> {
    if text.trim().is_empty() {
        bail!("text is empty — nothing to speak");
    }
    if voice.trim().is_empty() {
        bail!("voice is empty — set media.tts_voice in nami.json or pass voice_id");
    }

    let body = build_request_body(text, voice);
    let response: GenerationResponse =
        crate::media::post_generate_content(api_key, model, &body, timeout).await?;

    let Some(candidate) = response.candidates.first() else {
        bail!(
            "model returned no candidates (promptFeedback: {:?})",
            response.prompt_feedback
        );
    };

    let mut speech: Vec<GeneratedSpeech> = Vec::new();
    for part in candidate.content.parts.clone().unwrap_or_default() {
        if let Part::InlineData { inline_data } = part {
            let data = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                &inline_data.data,
            )
            .context("audio inlineData is not valid base64")?;
            speech.push(GeneratedSpeech {
                mime_type: inline_data.mime_type,
                data,
            });
        }
    }

    if speech.is_empty() {
        bail!(
            "model returned no audio data — check that {} supports audio output",
            model
        );
    }
    Ok(speech)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Request body must contain responseModalities ["AUDIO"] + correct speechConfig.voiceConfig
    #[test]
    fn test_build_request_body() {
        let body = build_request_body("Hello from Nami.", "Kore");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "Hello from Nami.");
        assert_eq!(
            body["generationConfig"]["responseModalities"],
            serde_json::json!(["AUDIO"])
        );
        assert_eq!(
            body["generationConfig"]["speechConfig"]["voiceConfig"]["prebuiltVoiceConfig"]["voiceName"],
            "Kore"
        );
    }

    /// Emotion prepended as hint before text; unassigned = original text
    #[test]
    fn test_compose_speech_text() {
        assert_eq!(compose_speech_text("hi", None), "hi");
        assert_eq!(compose_speech_text("hi", Some("  ")), "hi");
        let composed = compose_speech_text("Hello.", Some("cheerful"));
        assert!(composed.starts_with("Deliver the following text in a cheerful tone."));
        assert!(composed.ends_with("Hello."));
    }

    /// MIME → extension (wav/mp3/raw) based on actual type
    #[test]
    fn test_extension_for_mime() {
        assert_eq!(extension_for_mime("audio/wav"), "wav");
        assert_eq!(extension_for_mime("audio/wav;rate=24000"), "wav");
        assert_eq!(extension_for_mime("audio/mpeg"), "mp3");
        assert_eq!(extension_for_mime("audio/L16;rate=24000"), "raw");
    }

    /// Estimate duration from valid WAV header / return None for invalid header
    #[test]
    fn test_estimate_wav_duration_secs() {
        // Mock WAV header: byteRate = 48000 (24kHz 16-bit mono), data 96000 bytes → 2s
        let mut wav = vec![0u8; 44 + 96_000];
        wav[0..4].copy_from_slice(b"RIFF");
        wav[8..12].copy_from_slice(b"WAVE");
        wav[28..32].copy_from_slice(&48_000u32.to_le_bytes());
        let d = estimate_wav_duration_secs(&wav).unwrap();
        assert!((d - 2.0).abs() < 0.001, "expected ~2.0s, got {d}");

        assert_eq!(estimate_wav_duration_secs(b"not a wav"), None);
        assert_eq!(estimate_wav_duration_secs(&[0u8; 10]), None);
        // RIFF but byteRate = 0 → prevents divide by zero
        let mut zero_rate = vec![0u8; 44];
        zero_rate[0..4].copy_from_slice(b"RIFF");
        zero_rate[8..12].copy_from_slice(b"WAVE");
        assert_eq!(estimate_wav_duration_secs(&zero_rate), None);
    }

    /// Empty text/voice rejected before API call
    #[tokio::test]
    async fn test_generate_rejects_empty_input() {
        let err = generate("k", "m", "  ", "Kore", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("text is empty"));

        let err = generate("k", "m", "hi", " ", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("voice is empty"));
    }

    /// Live round-trip with real API — requires GOOGLE_API_KEY
    /// Run with: `cargo test media::tts::tests::test_generate_live -- --ignored`
    #[tokio::test]
    #[ignore = "requires GOOGLE_API_KEY and makes a real (cheap) API call"]
    async fn test_generate_live() {
        // Test binary doesn't run main(), load .env manually
        let _ = dotenvy::dotenv();
        let Ok(key) = crate::media::require_api_key() else {
            eprintln!("skipping: GOOGLE_API_KEY not set");
            return;
        };
        let speech = generate(
            &key,
            "gemini-3.8-flash-tts",
            "Hello from Nami. Multimedia generation is online.",
            "Kore",
            Duration::from_secs(120),
        )
        .await
        .expect("live TTS generation");
        assert!(!speech.is_empty());
        assert!(
            speech[0].data.len() > 1_000,
            "audio bytes too small: {}",
            speech[0].data.len()
        );
        // Spike confirmed returns audio/wav (RIFF)
        let d = &speech[0].data;
        assert_eq!(&d[0..4], b"RIFF", "expected WAV container");
        let dur = estimate_wav_duration_secs(d).expect("parseable WAV header");
        assert!(dur > 0.5, "expected a non-trivial clip, got {dur}s");
    }
}