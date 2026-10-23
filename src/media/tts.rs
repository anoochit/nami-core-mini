//! เรียก Gemini TTS model ผ่าน `generateContent` + `responseModalities: AUDIO`
//!
//! spike (2026-10-23) ยืนยันว่า `gemini-3.8-flash-tts` คืน `audio/wav` ที่มี
//! header ครบ — บันทึกเป็นไฟล์ `.wav` ได้ทันที ไม่ต้องห่อด้วย WAV เอง
//! เสียงที่คืนเป็น binary — ห้ามส่ง base64 เข้า context ของ agent
//! (best practice #2 ของ `wiki/multi-model-generation.md`)

use adk_gemini::{GenerationResponse, Part};
use anyhow::{Context, bail};
use std::time::Duration;

/// ผลลัพธ์เสียงหนึ่งไฟล์จาก TTS model (ปกติการเรียกหนึ่งครั้งคืน 1 ไฟล์)
#[derive(Debug)]
pub struct GeneratedSpeech {
    /// MIME type ที่ API คืนมาจริง เช่น `audio/wav`
    pub mime_type: String,
    /// ข้อมูลเสียง (ถอด base64 แล้ว)
    pub data: Vec<u8>,
}

/// ประกอบ request body สำหรับ TTS
///
/// `emotion` เป็น delivery hint ที่ต่อหน้าข้อความ (Gemini TTS ไม่มี
/// พารามิเตอร์ style ตรง ๆ — ทิศทางการพูดใส่ใน prompt ได้ตามธรรมชาติของ model)
///
/// แยกเป็น pure function เพื่อให้ทดสอบได้โดยไม่ต้องยิง API — รูปแบบผ่าน
/// spike จริงแล้วเมื่อ 2026-10-23 (voice `Kore` ได้ `audio/wav` 203 KB)
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

/// ประกอบข้อความที่จะพูด + delivery hint (อารมณ์/น้ำเสียง)
///
/// hint อยู่หน้าข้อความจริง เช่น
/// `Deliver the following in a cheerful tone.\n\nHello from Nami.`
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

/// MIME type ของผลลัพธ์ → นามสกุลไฟล์
///
/// model คืน `audio/wav` ในปัจจุบัน — ชนิดอื่น (PCM ดิบ/mp3) map ตามที่ API
/// คืนมาจริง ไม่เดาเอง
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

/// ประมาณความยาวเสียง (วินาที) จาก WAV header มาตรฐาน (RIFF/PCM)
///
/// อ่าน `byteRate` (offset 28, u32 LE) แล้วหารขนาดข้อมูล — คืน `None` ถ้า
/// ไม่ใช่ WAV ที่อ่าน header ได้ (เช่น PCM ดิบ)
pub fn estimate_wav_duration_secs(data: &[u8]) -> Option<f64> {
    if data.len() < 44 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }
    let byte_rate = u32::from_le_bytes([data[28], data[29], data[30], data[31]]);
    if byte_rate == 0 {
        return None;
    }
    // header มาตรฐาน PCM ใช้ 44 bytes (RIFF 12 + fmt 24 + data 8)
    let audio_bytes = data.len().saturating_sub(44) as f64;
    Some(audio_bytes / byte_rate as f64)
}

/// เรียก TTS model หนึ่งครั้ง แล้วคืนไฟล์เสียงทุก candidate (ปกติ 1 ไฟล์)
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

    /// body ต้องมี responseModalities ["AUDIO"] + speechConfig.voiceConfig ที่ถูกต้อง
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

    /// emotion ถูกต่อเป็น hint หน้าข้อความ; ไม่ระบุ = ข้อความเดิม
    #[test]
    fn test_compose_speech_text() {
        assert_eq!(compose_speech_text("hi", None), "hi");
        assert_eq!(compose_speech_text("hi", Some("  ")), "hi");
        let composed = compose_speech_text("Hello.", Some("cheerful"));
        assert!(composed.starts_with("Deliver the following text in a cheerful tone."));
        assert!(composed.ends_with("Hello."));
    }

    /// MIME → นามสกุล (wav/mp3/raw) ตาม type จริง
    #[test]
    fn test_extension_for_mime() {
        assert_eq!(extension_for_mime("audio/wav"), "wav");
        assert_eq!(extension_for_mime("audio/wav;rate=24000"), "wav");
        assert_eq!(extension_for_mime("audio/mpeg"), "mp3");
        assert_eq!(extension_for_mime("audio/L16;rate=24000"), "raw");
    }

    /// ประมาณความยาวจาก WAV header ที่ถูกต้อง / คืน None สำหรับของปลอม
    #[test]
    fn test_estimate_wav_duration_secs() {
        // WAV header สมมติ: byteRate = 48000 (24kHz 16-bit mono), data 96000 bytes → 2s
        let mut wav = vec![0u8; 44 + 96_000];
        wav[0..4].copy_from_slice(b"RIFF");
        wav[8..12].copy_from_slice(b"WAVE");
        wav[28..32].copy_from_slice(&48_000u32.to_le_bytes());
        let d = estimate_wav_duration_secs(&wav).unwrap();
        assert!((d - 2.0).abs() < 0.001, "expected ~2.0s, got {d}");

        assert_eq!(estimate_wav_duration_secs(b"not a wav"), None);
        assert_eq!(estimate_wav_duration_secs(&[0u8; 10]), None);
        // RIFF แต่ byteRate = 0 → ป้องกันหารด้วยศูนย์
        let mut zero_rate = vec![0u8; 44];
        zero_rate[0..4].copy_from_slice(b"RIFF");
        zero_rate[8..12].copy_from_slice(b"WAVE");
        assert_eq!(estimate_wav_duration_secs(&zero_rate), None);
    }

    /// text/voice ว่างต้องถูกปฏิเสธก่อนยิง API (ไม่ต้องใช้คีย์)
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

    /// round-trip กับ API จริง — ต้องมี GOOGLE_API_KEY และเสียเงินเล็กน้อย
    /// รันด้วย: `cargo test media::tts::tests::test_generate_live -- --ignored`
    #[tokio::test]
    #[ignore = "requires GOOGLE_API_KEY and makes a real (cheap) API call"]
    async fn test_generate_live() {
        // test binary ไม่ได้รัน main() จึงต้องโหลด .env เอง (เหมือนที่ nami จริงทำ)
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
        // spike ยืนยันว่าคืน audio/wav (RIFF)
        let d = &speech[0].data;
        assert_eq!(&d[0..4], b"RIFF", "expected WAV container");
        let dur = estimate_wav_duration_secs(d).expect("parseable WAV header");
        assert!(dur > 0.5, "expected a non-trivial clip, got {dur}s");
    }
}
