//! Media toolset (Gemini image/TTS/video) for sub-agents.
//!
//! Each tool writes files to `media/run-<ts>-<id>/` with `manifest.json`
//! (traceability) and returns **path + metadata only** — no base64 returned into
//! model context. See plan in `wiki/multimodal-generation-plan.md`.

use crate::config::{MediaConfig, NamiConfig};
use crate::media::{self, image, music, tts, video};
use crate::workspace::WorkspaceGuard;
use adk_rust::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

/// Parameters for `generate_image`
#[derive(JsonSchema, Serialize, Deserialize, Debug)]
pub struct GenerateImageParams {
    /// Detailed image description (English works best), e.g. 'A minimalist line drawing of an ocean wave, white background'
    pub prompt: String,
    /// Aspect ratio: 1:1, 2:3, 3:2, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9, 21:9 (optional)
    pub aspect_ratio: Option<String>,
    /// Style hint appended to the prompt, e.g. 'watercolor', 'pixel art' (optional)
    pub style_preset: Option<String>,
}

/// Parses raw args (JSON from model) into a struct and validates.
///
/// Extracted as a pure function for testability without creating a tool.
/// Note: Uses full `std::result::Result` because ADK prelude shadows `Result`
/// with `Result<T, AdkError>`.
fn parse_image_args(args: &Value) -> std::result::Result<GenerateImageParams, String> {
    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Missing required argument: prompt".to_string())?
        .to_string();

    let aspect_ratio = match args.get("aspect_ratio") {
        Some(Value::String(s)) if !s.trim().is_empty() => {
            let ar = s.trim().to_string();
            if !image::SUPPORTED_ASPECT_RATIOS.contains(&ar.as_str()) {
                return Err(format!(
                    "Unsupported aspect_ratio {ar:?} — supported: {}",
                    image::SUPPORTED_ASPECT_RATIOS.join(", ")
                ));
            }
            Some(ar)
        }
        _ => None,
    };

    let style_preset = match args.get("style_preset") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    };

    Ok(GenerateImageParams {
        prompt,
        aspect_ratio,
        style_preset,
    })
}

/// Composes the final prompt: appends style hint to the end of prompt.
///
/// Gemini image has no native style parameter — style_preset is a hint appended
/// to prompt (pure function for testability).
fn compose_prompt(prompt: &str, style_preset: Option<&str>) -> String {
    match style_preset {
        Some(style) => format!("{prompt}\n\nStyle: {style}."),
        None => prompt.to_string(),
    }
}

/// Creates `generate_image` tool — calls Gemini image model and saves to disk.
pub fn create_image_tool(cfg: MediaConfig) -> Arc<dyn Tool> {
    let tool = FunctionTool::new(
        "generate_image",
        "Generate an image with a Gemini image model. Saves the image file to disk \
         and returns its path + metadata (never base64). Use aspect_ratio like '16:9' \
         for wide scenes or '9:16' for vertical clips.",
        move |_ctx, args| {
            let cfg = cfg.clone();
            async move {
                let params = match parse_image_args(&args) {
                    Ok(p) => p,
                    Err(msg) => return Ok(json!({ "error": msg })),
                };
                let api_key = match media::require_api_key() {
                    Ok(k) => k,
                    Err(e) => return Ok(json!({ "error": e.to_string() })),
                };

                let started = std::time::Instant::now();
                let run_dir = match media::create_run_dir(&media::output_base(&cfg)) {
                    Ok(d) => d,
                    Err(e) => return Ok(json!({ "error": format!("cannot create run dir: {e}") })),
                };
                let mut manifest = media::manifest::Manifest::start(
                    "generate_image",
                    &cfg.image_model,
                    &params.prompt,
                    json!({
                        "aspect_ratio": params.aspect_ratio,
                        "style_preset": params.style_preset,
                    }),
                );

                let prompt = compose_prompt(&params.prompt, params.style_preset.as_deref());
                let outcome = image::generate(
                    &api_key,
                    &cfg.image_model,
                    &prompt,
                    params.aspect_ratio.as_deref(),
                    std::time::Duration::from_secs(cfg.timeout_secs),
                )
                .await;

                match outcome {
                    Ok(images) => {
                        let mut outputs = Vec::new();
                        for (i, img) in images.iter().enumerate() {
                            let ext = image::extension_for_mime(&img.mime_type);
                            let file_name = format!("image-{}.{}", i + 1, ext);
                            let file_path = run_dir.join(&file_name);
                            if let Err(e) = std::fs::write(&file_path, &img.data) {
                                manifest.finish_err(&format!("cannot write {}: {e}", file_path.display()));
                                let _ = manifest.save(&run_dir);
                                return Ok(json!({ "error": format!("cannot write {}: {e}", file_path.display()) }));
                            }
                            manifest.add_output(&file_name, &img.mime_type, img.data.len() as u64);
                            outputs.push(json!({
                                "path": file_path.display().to_string(),
                                "mime_type": img.mime_type,
                                "bytes": img.data.len(),
                            }));
                        }
                        manifest.finish_ok();
                        let _ = manifest.save(&run_dir);
                        let mut result = json!({
                            "status": "ok",
                            "model": cfg.image_model,
                            "run_dir": run_dir.display().to_string(),
                            "outputs": outputs,
                            "duration_ms": started.elapsed().as_millis() as u64,
                            "note": "Files are saved on disk. Reference them by path in later \
                                     calls (e.g. generate_video) — never request their bytes \
                                     back into the conversation.",
                        });
                        // Accompanying text of first file (if model returned one)
                        if let Some(text) = images.first().and_then(|i| i.text.as_deref()) {
                            result["text"] = json!(text);
                        }
                        Ok(result)
                    }
                    Err(e) => {
                        manifest.finish_err(&e.to_string());
                        let _ = manifest.save(&run_dir);
                        Ok(json!({
                            "error": e.to_string(),
                            "run_dir": run_dir.display().to_string(),
                        }))
                    }
                }
            }
        },
    )
    // Media tools write separate files per call — safe for parallel execution
    .with_concurrency_safe(true)
    .with_parameters_schema::<GenerateImageParams>();

    Arc::new(tool)
}

/// Parameters for `generate_tts`
#[derive(JsonSchema, Serialize, Deserialize, Debug)]
pub struct GenerateTtsParams {
    /// The text to speak. Keep each clip short (1-3 sentences) so scene audio stays in sync.
    pub text: String,
    /// Prebuilt Gemini voice name, e.g. 'Kore', 'Puck', 'Charon' (optional; defaults to the configured voice)
    pub voice_id: Option<String>,
    /// Delivery tone hint, e.g. 'cheerful', 'calm', 'dramatic' (optional)
    pub emotion: Option<String>,
    /// Output format: 'wav' (native, default) or 'mp3' (converted locally via ffmpeg; requires ffmpeg in PATH)
    pub output_format: Option<String>,
}

/// Parses raw args (JSON from model) into a struct and validates.
///
/// Note: Uses full `std::result::Result` because ADK prelude shadows `Result`
/// with `Result<T, AdkError>`.
fn parse_tts_args(args: &Value) -> std::result::Result<GenerateTtsParams, String> {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Missing required argument: text".to_string())?
        .to_string();

    let voice_id = match args.get("voice_id") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    };
    let emotion = match args.get("emotion") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    };

    // output_format: "wav" (native) or "mp3" (converted locally via ffmpeg)
    let output_format = match args.get("output_format") {
        Some(Value::String(s)) => {
            let fmt = s.trim().to_ascii_lowercase();
            match fmt.as_str() {
                "wav" | "mp3" => Some(fmt),
                other => {
                    return Err(format!(
                        "unsupported output_format '{other}' — use 'wav' or 'mp3'"
                    ));
                }
            }
        }
        _ => None,
    };

    Ok(GenerateTtsParams {
        text,
        voice_id,
        emotion,
        output_format,
    })
}

/// Creates `generate_tts` tool — calls Gemini TTS model and saves to disk.
///
/// Does not whitelist voice names as Google updates them often — lets API
/// return error on invalid voice.
pub fn create_tts_tool(cfg: MediaConfig) -> Arc<dyn Tool> {
    let tool = FunctionTool::new(
        "generate_tts",
        "Synthesize speech with a Gemini TTS model. Saves the audio file to disk \
         and returns its path + metadata (never base64). Defaults to WAV; pass \
         output_format='mp3' to convert locally with ffmpeg (requires ffmpeg in \
         PATH). Optional emotion sets the delivery tone. voice_id overrides the \
         default voice.",
        move |_ctx, args| {
            let cfg = cfg.clone();
            async move {
                let params = match parse_tts_args(&args) {
                    Ok(p) => p,
                    Err(msg) => return Ok(json!({ "error": msg })),
                };
                let api_key = match media::require_api_key() {
                    Ok(k) => k,
                    Err(e) => return Ok(json!({ "error": e.to_string() })),
                };
                let voice = params.voice_id.clone().unwrap_or_else(|| cfg.tts_voice.clone());

                let started = std::time::Instant::now();
                let run_dir = match media::create_run_dir(&media::output_base(&cfg)) {
                    Ok(d) => d,
                    Err(e) => return Ok(json!({ "error": format!("cannot create run dir: {e}") })),
                };
                let mut manifest = media::manifest::Manifest::start(
                    "generate_tts",
                    &cfg.tts_model,
                    &params.text,
                    json!({
                        "voice_id": voice,
                        "emotion": params.emotion,
                    }),
                );

                let speech_text = tts::compose_speech_text(&params.text, params.emotion.as_deref());
                let outcome = tts::generate(
                    &api_key,
                    &cfg.tts_model,
                    &speech_text,
                    &voice,
                    std::time::Duration::from_secs(cfg.timeout_secs),
                )
                .await;

                match outcome {
                    Ok(clips) => {
                        let want_mp3 = params.output_format.as_deref() == Some("mp3");
                        if want_mp3
                            && let Err(e) = media::assemble::ensure_ffmpeg().await
                        {
                            let msg = format!("output_format=mp3 requires ffmpeg: {e}");
                            manifest.finish_err(&msg);
                            let _ = manifest.save(&run_dir);
                            return Ok(json!({ "error": msg }));
                        }
                        let mut outputs = Vec::new();
                        for (i, clip) in clips.iter().enumerate() {
                            let ext = tts::extension_for_mime(&clip.mime_type);
                            let mut out_name = format!("speech-{}.{}", i + 1, ext);
                            let mut out_path = run_dir.join(&out_name);
                            let mut out_mime = clip.mime_type.clone();
                            if let Err(e) = std::fs::write(&out_path, &clip.data) {
                                manifest
                                    .finish_err(&format!("cannot write {}: {e}", out_path.display()));
                                let _ = manifest.save(&run_dir);
                                return Ok(json!({ "error": format!("cannot write {}: {e}", out_path.display()) }));
                            }
                            let duration = tts::estimate_wav_duration_secs(&clip.data);

                            // output_format=mp3: convert freshly written file to mp3
                            // and remove original wav (manifest stores mp3 only)
                            let mut out_bytes = clip.data.len() as u64;
                            if want_mp3 {
                                let mp3_path = run_dir.join(format!("speech-{}.mp3", i + 1));
                                if let Err(e) = media::assemble::convert_to_mp3(
                                    &out_path,
                                    &mp3_path,
                                    std::time::Duration::from_secs(cfg.timeout_secs),
                                )
                                .await
                                {
                                    let msg = format!("mp3 conversion failed: {e}");
                                    manifest.finish_err(&msg);
                                    let _ = manifest.save(&run_dir);
                                    return Ok(json!({ "error": msg }));
                                }
                                let _ = std::fs::remove_file(&out_path);
                                out_bytes = std::fs::metadata(&mp3_path)
                                    .map(|m| m.len())
                                    .unwrap_or(0);
                                out_name = format!("speech-{}.mp3", i + 1);
                                out_path = mp3_path;
                                out_mime = "audio/mpeg".to_string();
                            }

                            manifest
                                .add_output(&out_name, &out_mime, out_bytes)
                                .duration_seconds = duration;
                            outputs.push(json!({
                                "path": out_path.display().to_string(),
                                "mime_type": out_mime,
                                "bytes": out_bytes,
                                "duration_seconds": duration,
                            }));
                        }
                        manifest.finish_ok();
                        let _ = manifest.save(&run_dir);
                        Ok(json!({
                            "status": "ok",
                            "model": cfg.tts_model,
                            "voice": voice,
                            "run_dir": run_dir.display().to_string(),
                            "outputs": outputs,
                            "duration_ms": started.elapsed().as_millis() as u64,
                            "note": "Audio files are saved on disk. Reference them by path — \
                                     never request their bytes back into the conversation.",
                        }))
                    }
                    Err(e) => {
                        manifest.finish_err(&e.to_string());
                        let _ = manifest.save(&run_dir);
                        Ok(json!({
                            "error": e.to_string(),
                            "run_dir": run_dir.display().to_string(),
                        }))
                    }
                }
            }
        },
    )
    // Media tools write separate files per call — safe for parallel execution
    .with_concurrency_safe(true)
    .with_parameters_schema::<GenerateTtsParams>();

    Arc::new(tool)
}

/// Parameters for `generate_music`
#[derive(JsonSchema, Serialize, Deserialize, Debug)]
pub struct GenerateMusicParams {
    /// What the music should sound like: mood, instrumentation, tempo, scene context
    pub prompt: String,
    /// Genre/style hint merged into the prompt, e.g. 'lo-fi hip hop', 'orchestral' (optional)
    pub style: Option<String>,
}

/// Parses raw args (JSON from model) into a struct and validates.
///
/// Note: Uses full `std::result::Result` because ADK prelude shadows `Result`
/// with `Result<T, AdkError>`.
fn parse_music_args(args: &Value) -> std::result::Result<GenerateMusicParams, String> {
    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Missing required argument: prompt".to_string())?
        .to_string();

    let style = match args.get("style") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    };

    Ok(GenerateMusicParams { prompt, style })
}

/// Creates `generate_music` tool — calls Lyria music model and saves to disk.
///
/// No length/tempo parameter control — model returns fixed ~30s clip (spikes confirm
/// `durationSeconds` returns 400 rejection); style is adjusted via prompt.
pub fn create_music_tool(cfg: MediaConfig) -> Arc<dyn Tool> {
    let tool = FunctionTool::new(
        "generate_music",
        "Generate a ~30 second instrumental music track with a Gemini music model. \
         Saves the audio file (MP3) to disk and returns its path + metadata (never \
         base64). Describe mood, instrumentation and tempo in the prompt; use \
         style for a genre hint. Clip length is fixed by the model.",
        move |_ctx, args| {
            let cfg = cfg.clone();
            async move {
                let params = match parse_music_args(&args) {
                    Ok(p) => p,
                    Err(msg) => return Ok(json!({ "error": msg })),
                };
                let api_key = match media::require_api_key() {
                    Ok(k) => k,
                    Err(e) => return Ok(json!({ "error": e.to_string() })),
                };

                let started = std::time::Instant::now();
                let run_dir = match media::create_run_dir(&media::output_base(&cfg)) {
                    Ok(d) => d,
                    Err(e) => return Ok(json!({ "error": format!("cannot create run dir: {e}") })),
                };
                let mut manifest = media::manifest::Manifest::start(
                    "generate_music",
                    &cfg.music_model,
                    &params.prompt,
                    json!({ "style": params.style }),
                );

                let music_prompt = music::compose_music_prompt(&params.prompt, params.style.as_deref());
                let outcome = music::generate(
                    &api_key,
                    &cfg.music_model,
                    &music_prompt,
                    std::time::Duration::from_secs(cfg.timeout_secs),
                )
                .await;

                match outcome {
                    Ok(track) => {
                        let ext = music::extension_for_mime(&track.mime_type);
                        let file_name = format!("music-1.{ext}");
                        let file_path = run_dir.join(&file_name);
                        if let Err(e) = std::fs::write(&file_path, &track.data) {
                            manifest
                                .finish_err(&format!("cannot write {}: {e}", file_path.display()));
                            let _ = manifest.save(&run_dir);
                            return Ok(json!({ "error": format!("cannot write {}: {e}", file_path.display()) }));
                        }
                        manifest.add_output(&file_name, &track.mime_type, track.data.len() as u64);
                        manifest.finish_ok();
                        let _ = manifest.save(&run_dir);
                        Ok(json!({
                            "status": "ok",
                            "model": cfg.music_model,
                            "run_dir": run_dir.display().to_string(),
                            "outputs": [json!({
                                "path": file_path.display().to_string(),
                                "mime_type": track.mime_type,
                                "bytes": track.data.len(),
                            })],
                            "duration_ms": started.elapsed().as_millis() as u64,
                            "note": "Music is a fixed ~30s clip. The file is saved on disk — \
                                     reference it by path, never request its bytes back \
                                     into the conversation.",
                        }))
                    }
                    Err(e) => {
                        manifest.finish_err(&e.to_string());
                        let _ = manifest.save(&run_dir);
                        Ok(json!({
                            "error": e.to_string(),
                            "run_dir": run_dir.display().to_string(),
                        }))
                    }
                }
            }
        },
    )
    // Media tools write separate files per call — safe for parallel execution
    .with_concurrency_safe(true)
    .with_parameters_schema::<GenerateMusicParams>();

    Arc::new(tool)
}

/// Parameters for `generate_video`
#[derive(JsonSchema, Serialize, Deserialize, Debug)]
pub struct GenerateVideoParams {
    /// Cinematic scene description, e.g. 'A calm ocean wave rolling at golden hour, slow motion'
    pub prompt: String,
    /// Clip length in seconds: 4, 6 or 8 (optional; default 8)
    pub duration_seconds: Option<u32>,
    /// Aspect ratio: '16:9' (widescreen) or '9:16' (vertical) (optional)
    pub aspect_ratio: Option<String>,
    /// Optional workspace path to a reference image (e.g. output of generate_image) used as the first frame
    pub reference_image_path: Option<String>,
}

/// Supported values for `generate_video` duration/aspect.
///
/// Current video model (`gemini-omni-1.1-flash`) has no config fields for these,
/// so they are passed as prompt hints via `video::compose_video_prompt` — this whitelist
/// conveys intent and provides fast feedback to model before calling API.
const VIDEO_DURATIONS: &[u32] = &[4, 6, 8];
const VIDEO_ASPECT_RATIOS: &[&str] = &["16:9", "9:16"];

/// Parses raw args (JSON from model) into a struct and validates duration/aspect.
///
/// Note: Uses full `std::result::Result` because ADK prelude shadows `Result`
/// with `Result<T, AdkError>`.
fn parse_video_args(args: &Value) -> std::result::Result<GenerateVideoParams, String> {
    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Missing required argument: prompt".to_string())?
        .to_string();

    // Some models send "8" as string — accept both number and numeric string
    let duration_seconds = match args.get("duration_seconds") {
        Some(Value::Number(n)) => n.as_u64().map(|v| v as u32),
        Some(Value::String(s)) => s.trim().parse::<u32>().ok(),
        _ => None,
    };
    if let Some(raw) = args.get("duration_seconds")
        && !raw.is_null()
        && duration_seconds.is_none()
    {
        return Err("duration_seconds must be an integer: 4, 6 or 8".to_string());
    }

    let aspect_ratio = match args.get("aspect_ratio") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    };
    let reference_image_path = match args.get("reference_image_path") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    };

    if let Some(d) = duration_seconds
        && !VIDEO_DURATIONS.contains(&d)
    {
        return Err(format!(
            "unsupported duration_seconds {d} — supported: {}",
            VIDEO_DURATIONS
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(ar) = &aspect_ratio
        && !VIDEO_ASPECT_RATIOS.contains(&ar.as_str())
    {
        return Err(format!(
            "unsupported aspect_ratio {ar:?} — supported: {}",
            VIDEO_ASPECT_RATIOS.join(", ")
        ));
    }

    Ok(GenerateVideoParams {
        prompt,
        duration_seconds,
        aspect_ratio,
        reference_image_path,
    })
}

/// MIME type for reference image from extension (video model accepts image/png|jpeg|webp)
fn mime_for_reference_image(path: &std::path::Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

/// Creates `generate_video` tool — creates video interaction and waits for clip completion.
///
/// Uses Interactions API of `gemini-omni-1.1-flash` (see `src/media/video.rs`) —
/// this call takes minutes (waiting for server render) — periodically check
/// `ctx.is_cancelled()` during polling and bound by `media.timeout_secs`.
pub fn create_video_tool(
    media_cfg: MediaConfig,
    allowed_paths: Vec<std::path::PathBuf>,
    read_only: bool,
) -> Arc<dyn Tool> {
    let tool = FunctionTool::new(
        "generate_video",
        "Generate a short video clip with the Gemini video model. The call takes \
         minutes (server-side rendering) and returns the saved clip path. Optionally \
         pass a reference image path (output of generate_image) as the first frame. \
         Clips are 4-8 seconds.",
        move |ctx, args| {
            let media_cfg = media_cfg.clone();
            let allowed_paths = allowed_paths.clone();
            async move {
                let params = match parse_video_args(&args) {
                    Ok(p) => p,
                    Err(msg) => return Ok(json!({ "error": msg })),
                };
                let api_key = match media::require_api_key() {
                    Ok(k) => k,
                    Err(e) => return Ok(json!({ "error": e.to_string() })),
                };

                // reference image: validate with workspace guard then read + base64
                let reference: Option<(String, String)> = match &params.reference_image_path {
                    Some(p) => {
                        let guard = match WorkspaceGuard::new(allowed_paths.clone(), read_only) {
                            Ok(g) => g,
                            Err(e) => return Ok(json!({ "error": format!("workspace guard: {e}") })),
                        };
                        let path = match guard.validate_path(p, false) {
                            Ok(path) => path,
                            Err(e) => return Ok(json!({ "error": e.to_string() })),
                        };
                        let Some(mime) = mime_for_reference_image(&path) else {
                            return Ok(json!({ "error": format!(
                                "unsupported reference image type for {:?} — use .png, .jpg or .webp",
                                path.file_name().unwrap_or_default()
                            ) }));
                        };
                        match std::fs::read(&path) {
                            Ok(data) => {
                                let b64 = base64::Engine::encode(
                                    &base64::engine::general_purpose::STANDARD,
                                    &data,
                                );
                                Some((mime.to_string(), b64))
                            }
                            Err(e) => {
                                return Ok(json!({ "error": format!("cannot read reference image {}: {e}", path.display()) }));
                            }
                        }
                    }
                    None => None,
                };

                let started = std::time::Instant::now();
                let run_dir = match media::create_run_dir(&media::output_base(&media_cfg)) {
                    Ok(d) => d,
                    Err(e) => return Ok(json!({ "error": format!("cannot create run dir: {e}") })),
                };
                let mut manifest = media::manifest::Manifest::start(
                    "generate_video",
                    &media_cfg.video_model,
                    &params.prompt,
                    json!({
                        "duration_seconds": params.duration_seconds,
                        "aspect_ratio": params.aspect_ratio,
                        "reference_image_path": params.reference_image_path,
                    }),
                );

                let outcome = video::generate(
                    &api_key,
                    &media_cfg.video_model,
                    &params.prompt,
                    params.duration_seconds,
                    params.aspect_ratio.as_deref(),
                    reference.as_ref().map(|(m, b)| (m.as_str(), b.as_str())),
                    std::time::Duration::from_secs(media_cfg.poll_interval_secs),
                    std::time::Duration::from_secs(media_cfg.timeout_secs),
                    || ctx.is_cancelled(),
                    Some(ctx.as_ref()),
                )
                .await;

                match outcome {
                    Ok(videos) => {
                        let mut outputs = Vec::new();
                        for (i, video) in videos.iter().enumerate() {
                            let file_name = format!("video-{}.mp4", i + 1);
                            let file_path = run_dir.join(&file_name);
                            if let Err(e) = std::fs::write(&file_path, &video.data) {
                                manifest
                                    .finish_err(&format!("cannot write {}: {e}", file_path.display()));
                                let _ = manifest.save(&run_dir);
                                return Ok(json!({ "error": format!("cannot write {}: {e}", file_path.display()) }));
                            }
                            manifest.add_output(&file_name, &video.mime_type, video.data.len() as u64);
                            outputs.push(json!({
                                "path": file_path.display().to_string(),
                                "mime_type": video.mime_type,
                                "bytes": video.data.len(),
                            }));
                        }
                        manifest.finish_ok();
                        let _ = manifest.save(&run_dir);
                        Ok(json!({
                            "status": "ok",
                            "model": media_cfg.video_model,
                            "run_dir": run_dir.display().to_string(),
                            "outputs": outputs,
                            "duration_seconds": params.duration_seconds.unwrap_or(8),
                            "duration_ms": started.elapsed().as_millis() as u64,
                            "note": "Video files are saved on disk. Reference them by path — \
                                     never request their bytes back into the conversation.",
                        }))
                    }
                    Err(e) => {
                        manifest.finish_err(&e.to_string());
                        let _ = manifest.save(&run_dir);
                        Ok(json!({
                            "error": e.to_string(),
                            "run_dir": run_dir.display().to_string(),
                        }))
                    }
                }
            }
        },
    )
    .with_concurrency_safe(true)
    // Clips take minutes — description note tells model not to retry while waiting
    // (Note: runner has its own tool_timeout 300s — agent.rs sets higher than
    // media.timeout_secs when sub-agent has media tools)
    .with_long_running(true)
    .with_parameters_schema::<GenerateVideoParams>();

    Arc::new(tool)
}

/// Parameters for `assemble_audio_video`
#[derive(JsonSchema, Serialize, Deserialize, Debug)]
pub struct AssembleAudioVideoParams {
    /// Path to the source video file inside the workspace (output of generate_video or any MP4)
    pub video_path: String,
    /// Path to the narration/voiceover audio to overlay (optional; output of generate_tts)
    pub audio_path: Option<String>,
    /// Path to a background music track to mix under the narration (optional; output of generate_music)
    pub music_path: Option<String>,
    /// Background music volume 0.0-1.0 (optional; default 0.3 when mixed with other audio, 1.0 for music only)
    pub music_volume: Option<f32>,
    /// Keep the video's own audio track in the mix (optional; default false)
    pub keep_video_audio: Option<bool>,
}

/// Parses raw args (JSON from model) into a struct and validates.
///
/// Note: Uses full `std::result::Result` because ADK prelude shadows `Result`
/// with `Result<T, AdkError>`.
fn parse_assemble_args(args: &Value) -> std::result::Result<AssembleAudioVideoParams, String> {
    let video_path = args
        .get("video_path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Missing required argument: video_path".to_string())?
        .to_string();

    let optional_path = |key: &str| -> Option<String> {
        args.get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let audio_path = optional_path("audio_path");
    let music_path = optional_path("music_path");

    if audio_path.is_none() && music_path.is_none() && !keep_video_audio_of(args) {
        return Err(
            "nothing to assemble — provide audio_path and/or music_path, or set \
             keep_video_audio=true"
                .to_string(),
        );
    }

    let music_volume = match args.get("music_volume").and_then(Value::as_f64) {
        Some(v) if (0.0..=1.0).contains(&v) => Some(v as f32),
        Some(_) => return Err("music_volume must be between 0.0 and 1.0".to_string()),
        None => None,
    };

    Ok(AssembleAudioVideoParams {
        video_path,
        audio_path,
        music_path,
        music_volume,
        keep_video_audio: args.get("keep_video_audio").and_then(Value::as_bool),
    })
}

/// Reads keep_video_audio value from args (default false)
fn keep_video_audio_of(args: &Value) -> bool {
    args.get("keep_video_audio")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Creates `assemble_audio_video` tool — combines video + narration (+ music)
/// using local ffmpeg and saves to run dir + manifest.
///
/// Video duration is master: audio is padded (`apad`) or trimmed (`-shortest`)
/// to finish with video — output is always written to run dir (does not accept
/// output_path from model to avoid workspace path guessing).
pub fn create_assemble_tool(
    media_cfg: MediaConfig,
    allowed_paths: Vec<std::path::PathBuf>,
    read_only: bool,
) -> Arc<dyn Tool> {
    let tool = FunctionTool::new(
        "assemble_audio_video",
        "Assemble a final MP4 from generated media using local ffmpeg: overlay a \
         narration audio track and/or mix background music into a video clip. The \
         video's duration is the master (audio is padded or cut to fit). Requires \
         ffmpeg installed locally. The output is written to the run directory and \
         recorded in the manifest.",
        move |ctx, args| {
            let media_cfg = media_cfg.clone();
            let allowed_paths = allowed_paths.clone();
            async move {
                let params = match parse_assemble_args(&args) {
                    Ok(p) => p,
                    Err(msg) => return Ok(json!({ "error": msg })),
                };

                if let Err(e) = media::assemble::ensure_ffmpeg().await {
                    return Ok(json!({ "error": e.to_string() }));
                }

                // Validate all input paths using workspace guard
                let guard = match WorkspaceGuard::new(allowed_paths.clone(), read_only) {
                    Ok(g) => g,
                    Err(e) => return Ok(json!({ "error": format!("workspace guard: {e}") })),
                };
                let mut resolved: Vec<std::path::PathBuf> = Vec::new();
                for p in std::iter::once(&params.video_path)
                    .chain(params.audio_path.iter())
                    .chain(params.music_path.iter())
                {
                    match guard.validate_path(p, false) {
                        Ok(path) => {
                            if !path.is_file() {
                                return Ok(
                                    json!({ "error": format!("input file not found: {p}") }),
                                );
                            }
                            resolved.push(path);
                        }
                        Err(e) => return Ok(json!({ "error": e.to_string() })),
                    }
                }
                let video = resolved[0].clone();
                let narration = params.audio_path.as_ref().map(|_| resolved[1].clone());
                let music = match (&params.music_path, &narration) {
                    (Some(_), Some(_)) => Some(resolved[2].clone()),
                    (Some(_), None) => Some(resolved[1].clone()),
                    (None, _) => None,
                };

                // Music volume: default 0.3 when mixing with other sources, 1.0 when music only
                let keep_video_audio = params.keep_video_audio.unwrap_or(false);
                let music_volume =
                    params
                        .music_volume
                        .unwrap_or(if keep_video_audio || narration.is_some() {
                            0.3
                        } else {
                            1.0
                        });

                let started = std::time::Instant::now();
                let run_dir = match media::create_run_dir(&media::output_base(&media_cfg)) {
                    Ok(d) => d,
                    Err(e) => return Ok(json!({ "error": format!("cannot create run dir: {e}") })),
                };
                let mut manifest = media::manifest::Manifest::start(
                    "assemble_audio_video",
                    "ffmpeg",
                    &format!("assemble {}", video.display()),
                    json!({
                        "video_path": params.video_path,
                        "audio_path": params.audio_path,
                        "music_path": params.music_path,
                        "music_volume": music_volume,
                        "keep_video_audio": keep_video_audio,
                    }),
                );

                let out_path = run_dir.join("assembled-1.mp4");
                let ffmpeg_args =
                    media::assemble::build_ffmpeg_args(&media::assemble::AssembleInputs {
                        video: &video,
                        narration: narration.as_deref(),
                        music: music.as_deref(),
                        music_volume,
                        keep_video_audio,
                        output: &out_path,
                    });
                let outcome = media::assemble::run_ffmpeg(
                    &ffmpeg_args,
                    std::time::Duration::from_secs(media_cfg.timeout_secs),
                    || ctx.is_cancelled(),
                )
                .await;

                match outcome {
                    Ok(()) => {
                        let bytes = match std::fs::metadata(&out_path) {
                            Ok(m) => m.len(),
                            Err(e) => {
                                let msg = format!("cannot stat {}: {e}", out_path.display());
                                manifest.finish_err(&msg);
                                let _ = manifest.save(&run_dir);
                                return Ok(json!({ "error": msg }));
                            }
                        };
                        manifest.add_output("assembled-1.mp4", "video/mp4", bytes);
                        manifest.finish_ok();
                        let _ = manifest.save(&run_dir);
                        Ok(json!({
                            "status": "ok",
                            "engine": "ffmpeg",
                            "run_dir": run_dir.display().to_string(),
                            "outputs": [json!({
                                "path": out_path.display().to_string(),
                                "mime_type": "video/mp4",
                                "bytes": bytes,
                            })],
                            "duration_ms": started.elapsed().as_millis() as u64,
                            "note": "Assembled video is saved on disk. Reference it by \
                                     path — never request its bytes back into the \
                                     conversation.",
                        }))
                    }
                    Err(e) => {
                        manifest.finish_err(&e.to_string());
                        let _ = manifest.save(&run_dir);
                        Ok(json!({
                            "error": e.to_string(),
                            "run_dir": run_dir.display().to_string(),
                        }))
                    }
                }
            }
        },
    )
    // ffmpeg uses local CPU as a separate process — safe to run concurrently (writes to distinct run dir)
    .with_concurrency_safe(true)
    .with_parameters_schema::<AssembleAudioVideoParams>();

    Arc::new(tool)
}

/// Creates media toolset based on `media.enabled` in nami.json.
///
/// Returns empty vec when media is disabled — caller (agent.rs) will issue a warning.
pub fn create_media_toolset(config: &NamiConfig) -> Vec<Arc<dyn Tool>> {
    if !config.media.enabled {
        return vec![];
    }
    vec![
        create_image_tool(config.media.clone()),
        create_tts_tool(config.media.clone()),
        create_music_tool(config.media.clone()),
        create_video_tool(
            config.media.clone(),
            config.workspace.allowed_paths.clone(),
            config.workspace.read_only,
        ),
        create_assemble_tool(
            config.media.clone(),
            config.workspace.allowed_paths.clone(),
            config.workspace.read_only,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All args provided + valid aspect -> parse succeeds
    #[test]
    fn test_parse_image_args_full() {
        let args = json!({
            "prompt": "a wave",
            "aspect_ratio": "16:9",
            "style_preset": "watercolor"
        });
        let p = parse_image_args(&args).unwrap();
        assert_eq!(p.prompt, "a wave");
        assert_eq!(p.aspect_ratio.as_deref(), Some("16:9"));
        assert_eq!(p.style_preset.as_deref(), Some("watercolor"));
    }

    /// Empty/missing prompt = error; empty aspect/style string = treated as None
    #[test]
    fn test_parse_image_args_validation() {
        assert!(parse_image_args(&json!({})).is_err());
        assert!(parse_image_args(&json!({ "prompt": "  " })).is_err());

        let p =
            parse_image_args(&json!({ "prompt": "x", "aspect_ratio": "", "style_preset": " " }))
                .unwrap();
        assert!(p.aspect_ratio.is_none());
        assert!(p.style_preset.is_none());
    }

    /// Aspect ratio outside whitelist must error with supported options
    #[test]
    fn test_parse_image_args_rejects_bad_aspect() {
        let err = parse_image_args(&json!({ "prompt": "x", "aspect_ratio": "7:3" })).unwrap_err();
        assert!(err.contains("Unsupported aspect_ratio"));
        assert!(err.contains("16:9"));
    }

    /// Style hint appended to prompt / no style = original prompt
    #[test]
    fn test_compose_prompt() {
        assert_eq!(compose_prompt("a wave", None), "a wave");
        let styled = compose_prompt("a wave", Some("watercolor"));
        assert!(styled.starts_with("a wave"));
        assert!(styled.contains("Style: watercolor."));
    }

    /// All TTS args provided -> parse succeeds; missing voice/emotion = None
    #[test]
    fn test_parse_tts_args() {
        let p = parse_tts_args(&json!({ "text": "Hello from Nami." })).unwrap();
        assert_eq!(p.text, "Hello from Nami.");
        assert!(p.voice_id.is_none());
        assert!(p.emotion.is_none());

        let p = parse_tts_args(&json!({
            "text": "Hi", "voice_id": "Puck", "emotion": "cheerful "
        }))
        .unwrap();
        assert_eq!(p.voice_id.as_deref(), Some("Puck"));
        assert_eq!(p.emotion.as_deref(), Some("cheerful"));
    }

    /// Empty/missing text = error; empty voice/emotion string = treated as None
    #[test]
    fn test_parse_tts_args_validation() {
        assert!(parse_tts_args(&json!({})).is_err());
        assert!(parse_tts_args(&json!({ "text": "   " })).is_err());

        let p = parse_tts_args(&json!({ "text": "x", "voice_id": "", "emotion": " " })).unwrap();
        assert!(p.voice_id.is_none());
        assert!(p.emotion.is_none());
    }

    /// Media disabled = empty toolset; enabled = image + tts + music + video + assemble
    #[test]
    fn test_media_toolset_respects_enabled() {
        let base = r#"{
            "name": "n", "version": "1", "provider": "p", "model": "m",
            "system_instruction": "s",
            "workspace": { "allowed_paths": ["./"], "read_only": false },
            "storage": { "db_path": "./n.db", "wiki_dir": "./wiki" }
        }"#;
        let off: NamiConfig = serde_json::from_str(base).unwrap();
        assert!(create_media_toolset(&off).is_empty());

        let on: NamiConfig = serde_json::from_str(&format!(
            "{}, \"media\": {{ \"enabled\": true }} }}",
            base.trim_end().strip_suffix('}').unwrap()
        ))
        .unwrap();
        assert_eq!(
            create_media_toolset(&on).len(),
            5,
            "image + tts + music + video + assemble"
        );
    }

    /// Assemble args: audio_path only / music_path only / volume + keep
    #[test]
    fn test_parse_assemble_args() {
        let p = parse_assemble_args(&json!({
            "video_path": "media/run-1/video-1.mp4",
            "audio_path": "media/run-1/speech-1.wav"
        }))
        .unwrap();
        assert_eq!(p.video_path, "media/run-1/video-1.mp4");
        assert_eq!(p.audio_path.as_deref(), Some("media/run-1/speech-1.wav"));
        assert!(p.music_path.is_none());
        assert!(p.music_volume.is_none());

        // music only + volume + keep_video_audio
        let p = parse_assemble_args(&json!({
            "video_path": "v.mp4",
            "music_path": "m.mp3",
            "music_volume": 0.4,
            "keep_video_audio": true
        }))
        .unwrap();
        assert_eq!(p.music_volume, Some(0.4));
        assert_eq!(p.keep_video_audio, Some(true));

        // No audio at all = error
        let err = parse_assemble_args(&json!({ "video_path": "v.mp4" })).unwrap_err();
        assert!(err.contains("nothing to assemble"), "got: {err}");
        // Missing video_path = error
        let err = parse_assemble_args(&json!({ "music_path": "m.mp3" })).unwrap_err();
        assert!(err.contains("video_path"), "got: {err}");
        // Volume out of range = error
        let err = parse_assemble_args(&json!({
            "video_path": "v.mp4",
            "music_path": "m.mp3",
            "music_volume": 2.0
        }))
        .unwrap_err();
        assert!(err.contains("0.0 and 1.0"), "got: {err}");
    }

    /// TTS args: output_format checks case + rejects unknown values
    #[test]
    fn test_parse_tts_args_output_format() {
        let p = parse_tts_args(&json!({"text": "hi", "output_format": "MP3"})).unwrap();
        assert_eq!(p.output_format.as_deref(), Some("mp3"));
        let p = parse_tts_args(&json!({"text": "hi", "output_format": "wav"})).unwrap();
        assert_eq!(p.output_format.as_deref(), Some("wav"));
        let err = parse_tts_args(&json!({"text": "hi", "output_format": "flac"})).unwrap_err();
        assert!(err.contains("output_format"), "got: {err}");
    }

    /// Music args: full values, empty prompt (error), empty style (treated as None)
    #[test]
    fn test_parse_music_args() {
        let p = parse_music_args(&json!({
            "prompt": "warm acoustic background for a travel vlog",
            "style": "lo-fi hip hop"
        }))
        .unwrap();
        assert_eq!(p.prompt, "warm acoustic background for a travel vlog");
        assert_eq!(p.style.as_deref(), Some("lo-fi hip hop"));

        // style is optional
        let p = parse_music_args(&json!({ "prompt": "epic orchestral intro" })).unwrap();
        assert!(p.style.is_none());

        assert!(parse_music_args(&json!({ "prompt": " " })).is_err());
        assert!(parse_music_args(&json!({})).is_err());
        // empty/non-string style = None, not error
        let p = parse_music_args(&json!({ "prompt": "p", "style": "  " })).unwrap();
        assert!(p.style.is_none());
    }

    /// Video args: full values, number/string duration, non-whitelisted validation
    #[test]
    fn test_parse_video_args() {
        let p = parse_video_args(&json!({
            "prompt": "a wave at sunset",
            "duration_seconds": 6,
            "aspect_ratio": "9:16",
            "reference_image_path": "media/run-x/image-1.jpg"
        }))
        .unwrap();
        assert_eq!(p.prompt, "a wave at sunset");
        assert_eq!(p.duration_seconds, Some(6));
        assert_eq!(p.aspect_ratio.as_deref(), Some("9:16"));
        assert_eq!(
            p.reference_image_path.as_deref(),
            Some("media/run-x/image-1.jpg")
        );

        // numeric string also passes
        let p = parse_video_args(&json!({ "prompt": "x", "duration_seconds": "8" })).unwrap();
        assert_eq!(p.duration_seconds, Some(8));

        // empty prompt / non-numeric duration / duration outside whitelist / invalid aspect
        assert!(parse_video_args(&json!({ "prompt": " " })).is_err());
        let err =
            parse_video_args(&json!({ "prompt": "x", "duration_seconds": "long" })).unwrap_err();
        assert!(err.contains("must be an integer"));
        let err = parse_video_args(&json!({ "prompt": "x", "duration_seconds": 5 })).unwrap_err();
        assert!(err.contains("unsupported duration_seconds 5"));
        let err = parse_video_args(&json!({ "prompt": "x", "aspect_ratio": "1:1" })).unwrap_err();
        assert!(err.contains("unsupported aspect_ratio"));
    }

    /// Reference image: MIME from extension and rejects unsupported types
    #[test]
    fn test_mime_for_reference_image() {
        assert_eq!(
            mime_for_reference_image(std::path::Path::new("a/b.png")),
            Some("image/png")
        );
        assert_eq!(
            mime_for_reference_image(std::path::Path::new("b.JPG")),
            Some("image/jpeg")
        );
        assert_eq!(
            mime_for_reference_image(std::path::Path::new("c.webp")),
            Some("image/webp")
        );
        assert_eq!(
            mime_for_reference_image(std::path::Path::new("d.gif")),
            None
        );
        assert_eq!(
            mime_for_reference_image(std::path::Path::new("noext")),
            None
        );
    }
}
