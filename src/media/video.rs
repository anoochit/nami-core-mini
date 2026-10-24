//! Video generation via Gemini **Interactions API** (`gemini-omni-1.1-flash`)
//!
//! History (2026-10-23): Veo (`veo-3.1-generate-preview`) was decommissioned — even though
//! listed in model info, calling `:predictLongRunning` returns 404 "model is no longer available".
//! `gemini-omni-1.1-flash` (only working video model) exclusively supports Interactions API
//! (`generateContent` returns 400 "This model only supports Interactions API").
//!
//! adk-gemini 2.3.0 typed client supports this under feature `interactions`:
//! 1. `create interaction` (background) → returns interaction id immediately
//! 2. poll `get_interaction` until status is `completed`
//! 3. Extract `Content::Video` from `model_output` step — inline data (base64) or URI
//!
//! Model limitations: no `durationSeconds`/`aspectRatio` in config — passed via prompt hint
//! (`compose_video_prompt`) and recorded in manifest only.

use adk_gemini::Gemini;
use adk_gemini::interactions::{
    Content, CreateInteractionRequest, Input, Interaction, InteractionStatus, ResponseModality,
    Step,
};
use anyhow::{Context, bail};
use std::time::{Duration, Instant};

/// Single video clip retrieved from interaction (downloaded/decoded)
#[derive(Debug)]
pub struct GeneratedVideo {
    /// MIME type from content (default `video/mp4`)
    pub mime_type: String,
    /// Full video binary data
    pub data: Vec<u8>,
}

/// Raw video content reference from interaction — not yet decoded (pure data, testable)
#[derive(Debug, PartialEq)]
pub struct VideoRef {
    /// Inline base64 clip data — mutually exclusive with `uri`
    pub data_b64: Option<String>,
    /// Download URI (if model returns link)
    pub uri: Option<String>,
    /// MIME type returned by API (default `video/mp4`)
    pub mime_type: String,
}

/// Build request for Interactions API
///
/// If reference image exists = `Input::Content` (image block + text block)
/// Otherwise = standard `Input::Text` (pure function — testable)
pub fn build_request(
    model: &str,
    prompt: &str,
    reference_image: Option<(&str, &str)>,
) -> CreateInteractionRequest {
    let input = match reference_image {
        Some((mime, b64)) => Input::Content(vec![Content::image(b64, mime), Content::text(prompt)]),
        None => Input::Text(prompt.to_string()),
    };
    CreateInteractionRequest {
        model: Some(model.to_string()),
        input,
        response_modalities: vec![ResponseModality::Video],
        background: Some(true),
        ..Default::default()
    }
}

/// Append prompt hints for duration/aspect ratio (model lacks direct config parameters for these)
pub fn compose_video_prompt(
    prompt: &str,
    duration_seconds: Option<u32>,
    aspect_ratio: Option<&str>,
) -> String {
    let mut out = prompt.to_string();
    if let Some(d) = duration_seconds {
        out.push_str(&format!("\n\nThe clip must be exactly {d} seconds long."));
    }
    if let Some(ar) = aspect_ratio {
        out.push_str(&format!("\n\nFrame it in {ar} aspect ratio."));
    }
    out
}

/// Extract all video content blocks from interaction (skipping other steps)
///
/// Supports both inline (`data`) and URI — pure testable function
pub fn videos_from_interaction(interaction: &Interaction) -> Vec<VideoRef> {
    let mut refs = Vec::new();
    for step in &interaction.steps {
        if let Step::ModelOutput { content } = step {
            for block in content {
                if let Content::Video(v) = block {
                    refs.push(VideoRef {
                        data_b64: v.data.clone(),
                        uri: v.uri.clone(),
                        mime_type: v
                            .mime_type
                            .clone()
                            .unwrap_or_else(|| "video/mp4".to_string()),
                    });
                }
            }
        }
    }
    refs
}

/// Check if terminal interaction status is a failure (not completed)
fn failure_reason(interaction: &Interaction) -> Option<String> {
    match interaction.status {
        InteractionStatus::Completed => None,
        status => {
            let dump = serde_json::to_string(interaction).unwrap_or_default();
            Some(format!(
                "video interaction ended with status {:?} ({})",
                status,
                crate::media::truncate_for_error(&dump)
            ))
        }
    }
}

/// Download clip from URI — attaches API key for Google hosts only
async fn download(uri: &str, api_key: &str, timeout: Duration) -> anyhow::Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .context("cannot build HTTP client")?;
    let mut req = client.get(uri);
    if crate::media::host_is_google(uri) {
        req = req.header("x-goog-api-key", api_key);
    }
    let resp = req
        .send()
        .await
        .with_context(|| format!("network error downloading {uri}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!(
            "download returned {status}: {}",
            crate::media::truncate_for_error(&body)
        );
    }
    let bytes = resp.bytes().await.context("cannot read video bytes")?;
    Ok(bytes.to_vec())
}

/// Call video model once: create (background) → poll until `completed` → extract clip
///
/// `is_cancelled` checked every poll loop — returns `true` = cancels client and server side
/// (best effort via `cancel_interaction`)
///
/// `progress` (`ToolContext`) emits status updates during poll — passes `None` when context
/// absent (e.g. live tests) — streams `"stdout"` per `ToolContext::emit_progress` convention
#[allow(clippy::too_many_arguments)]
pub async fn generate(
    api_key: &str,
    model: &str,
    prompt: &str,
    duration_seconds: Option<u32>,
    aspect_ratio: Option<&str>,
    reference_image: Option<(&str, &str)>,
    poll_interval: Duration,
    timeout: Duration,
    is_cancelled: impl Fn() -> bool,
    progress: Option<&dyn adk_rust::ToolContext>,
) -> anyhow::Result<Vec<GeneratedVideo>> {
    if prompt.trim().is_empty() {
        bail!("prompt is empty — describe the video scene");
    }

    // Build client against actual Google base URL and pass to core implementation —
    // allows tests to pass client pointing to mock server
    let client = Gemini::with_model(api_key, format!("models/{model}"))
        .with_context(|| format!("invalid video model {model}"))?;
    generate_with_client(
        &client,
        api_key,
        model,
        prompt,
        duration_seconds,
        aspect_ratio,
        reference_image,
        poll_interval,
        timeout,
        is_cancelled,
        progress,
    )
    .await
}

/// Core implementation of [`generate`] — accepts pre-constructed client
///
/// Flow: create (background) → poll until `completed` → extract/download clip
#[allow(clippy::too_many_arguments)]
async fn generate_with_client(
    client: &Gemini,
    api_key: &str,
    model: &str,
    prompt: &str,
    duration_seconds: Option<u32>,
    aspect_ratio: Option<&str>,
    reference_image: Option<(&str, &str)>,
    poll_interval: Duration,
    timeout: Duration,
    is_cancelled: impl Fn() -> bool,
    progress: Option<&dyn adk_rust::ToolContext>,
) -> anyhow::Result<Vec<GeneratedVideo>> {
    // 1. Create interaction (background) via typed client
    let effective_prompt = compose_video_prompt(prompt, duration_seconds, aspect_ratio);
    let request = build_request(model, &effective_prompt, reference_image);
    let created = client
        .send_interaction(request)
        .await
        .context("cannot create video interaction")?;
    let interaction_id = created.id.clone();
    if interaction_id.is_empty() {
        bail!(
            "interaction created without id: {}",
            crate::media::truncate_for_error(&serde_json::to_string(&created).unwrap_or_default())
        );
    }
    tracing::info!(interaction = %interaction_id, model, "video interaction created");
    emit_progress(
        progress,
        "video requested — the server is rendering now (this can take minutes)...",
    )
    .await;

    // 2. Poll until completed — rendering takes minutes, enforcement of deadline + cancellation
    let started = Instant::now();
    let deadline = started + timeout;
    let mut last_progress = started;
    let finished = loop {
        if is_cancelled() {
            // Cancel server side as well (best effort)
            let _ = client.cancel_interaction(&interaction_id).await;
            bail!(
                "video generation cancelled by caller (interaction {interaction_id} cancelled server-side)"
            );
        }
        if Instant::now() >= deadline {
            bail!(
                "video generation timed out after {}s (interaction {interaction_id} may still be running)",
                timeout.as_secs()
            );
        }
        tokio::time::sleep(poll_interval).await;

        let state = client
            .get_interaction(&interaction_id, false)
            .await
            .context("cannot poll video interaction")?;
        match state.status {
            InteractionStatus::InProgress => {
                tracing::debug!(interaction = %interaction_id, "video interaction still running");
                // Report status every ~30 seconds
                if last_progress.elapsed() >= Duration::from_secs(30) {
                    last_progress = Instant::now();
                    emit_progress(
                        progress,
                        &format!("still rendering — {}s elapsed", started.elapsed().as_secs()),
                    )
                    .await;
                }
            }
            InteractionStatus::Completed => break state,
            other => {
                bail!(
                    "video interaction ended with status {other:?} ({})",
                    crate::media::truncate_for_error(
                        &serde_json::to_string(&state).unwrap_or_default()
                    )
                );
            }
        }
    };

    // 3. Extract video from model_output step
    if let Some(reason) = failure_reason(&finished) {
        bail!(reason);
    }
    let refs = videos_from_interaction(&finished);
    if refs.is_empty() {
        bail!(
            "interaction completed but returned no video (interaction: {}) — the prompt \
             may have been filtered",
            crate::media::truncate_for_error(&finished.id)
        );
    }

    emit_progress(progress, "render complete — downloading clip(s)...").await;

    // 4. Decode inline base64 or download from URI
    let mut videos = Vec::new();
    for r in &refs {
        let data = match (&r.data_b64, &r.uri) {
            (Some(b64), _) => {
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                    .context("video inlineData is not valid base64")?
            }
            (None, Some(uri)) => download(uri, api_key, timeout).await?,
            (None, None) => bail!("video content has neither data nor uri"),
        };
        videos.push(GeneratedVideo {
            mime_type: r.mime_type.clone(),
            data,
        });
    }
    tracing::info!(clips = videos.len(), "video generation complete");
    emit_progress(progress, "download complete.").await;
    Ok(videos)
}

/// Emit progress via `ToolContext::emit_progress` (no-op when context absent)
async fn emit_progress(progress: Option<&dyn adk_rust::ToolContext>, message: &str) {
    if let Some(ctx) = progress {
        ctx.emit_progress("stdout", message).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Request: model + Video modality + background and standard text input
    #[test]
    fn test_build_request_text_input() {
        let req = build_request("models/gemini-omni-1.1-flash", "a wave", None);
        assert_eq!(req.model.as_deref(), Some("models/gemini-omni-1.1-flash"));
        assert_eq!(req.response_modalities, vec![ResponseModality::Video]);
        assert_eq!(req.background, Some(true));
        assert!(matches!(req.input, Input::Text(ref t) if t == "a wave"));
    }

    /// With reference image = Input::Content containing image block + text block
    #[test]
    fn test_build_request_with_reference_image() {
        let req = build_request("m", "animate", Some(("image/png", "aGVsbG8=")));
        match &req.input {
            Input::Content(blocks) => {
                assert_eq!(blocks.len(), 2);
                assert!(matches!(&blocks[0], Content::Image(i)
                    if i.data.as_deref() == Some("aGVsbG8=") && i.mime_type.as_deref() == Some("image/png")));
                assert_eq!(blocks[1].as_text(), Some("animate"));
            }
            other => panic!("expected Content input, got {other:?}"),
        }
    }

    /// Duration/aspect appended as prompt hints
    #[test]
    fn test_compose_video_prompt() {
        assert_eq!(compose_video_prompt("p", None, None), "p");
        let out = compose_video_prompt("p", Some(4), Some("9:16"));
        assert!(out.contains("exactly 4 seconds"));
        assert!(out.contains("9:16 aspect ratio"));
    }

    /// Extract video refs from model_output step supporting both inline data and URI
    #[test]
    fn test_videos_from_interaction() {
        let interaction: Interaction = serde_json::from_value(serde_json::json!({
            "id": "int_123",
            "status": "completed",
            "steps": [
                { "type": "user_input", "content": [
                    { "type": "text", "text": "make me a video" }
                ]},
                { "type": "model_output", "content": [
                    { "type": "video", "data": "aGVsbG8=", "mimeType": "video/mp4" },
                    { "type": "text", "text": "here you go" },
                    { "type": "video", "uri": "https://storage.googleapis.com/x.mp4" }
                ]}
            ]
        }))
        .unwrap();

        let refs = videos_from_interaction(&interaction);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].data_b64.as_deref(), Some("aGVsbG8="));
        assert_eq!(refs[0].mime_type, "video/mp4");
        assert!(refs[0].uri.is_none());
        assert_eq!(
            refs[1].uri.as_deref(),
            Some("https://storage.googleapis.com/x.mp4")
        );
        assert_eq!(refs[1].mime_type, "video/mp4", "default mime applies");
    }

    /// Non-completed status returns failure_reason; completed returns None
    #[test]
    fn test_failure_reason() {
        let ok: Interaction = serde_json::from_value(serde_json::json!({
            "id": "i", "status": "completed", "steps": []
        }))
        .unwrap();
        assert!(failure_reason(&ok).is_none());

        let failed: Interaction = serde_json::from_value(serde_json::json!({
            "id": "i", "status": "failed", "steps": []
        }))
        .unwrap();
        let reason = failure_reason(&failed).expect("failure reason");
        assert!(reason.contains("Failed"));
    }

    /// Live round-trip with real API: create → poll → extract clip
    /// Run with: `cargo test media::video::tests::test_generate_live -- --ignored`
    #[tokio::test]
    #[ignore = "requires GOOGLE_API_KEY; makes a real PAID video call (minutes)"]
    async fn test_generate_live() {
        // Test binary doesn't run main(), load .env manually
        let _ = dotenvy::dotenv();
        let Ok(key) = crate::media::require_api_key() else {
            eprintln!("skipping: GOOGLE_API_KEY not set");
            return;
        };
        let videos = generate(
            &key,
            "gemini-omni-1.1-flash",
            "A calm ocean wave rolling onto a sandy beach at golden hour, cinematic, static camera",
            None,
            None,
            None,
            Duration::from_secs(5),
            Duration::from_secs(600),
            || false,
            None,
        )
        .await
        .expect("live video generation (create + poll + download)");
        assert!(!videos.is_empty());
        assert!(
            videos[0].data.len() > 100_000,
            "video too small: {} bytes",
            videos[0].data.len()
        );
        // mp4 has ftyp box at offset 4
        let d = &videos[0].data;
        if d.len() > 12 {
            assert_eq!(&d[4..8], b"ftyp", "expected an MP4 container");
        }
    }

    // ── Phase 8: Mock-HTTP tests for poll loop ──────

    use axum::extract::{Path as AxumPath, State as AxumState};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mock state for Interactions API
    struct MockApiState {
        script: Vec<serde_json::Value>,
        polls: AtomicUsize,
        cancels: AtomicUsize,
    }

    /// Create new interaction = always in_progress
    async fn mock_create(_body: Json<serde_json::Value>) -> Json<serde_json::Value> {
        Json(serde_json::json!({"id": "int_mock", "status": "in_progress", "steps": []}))
    }

    /// Poll = return response from script in order
    async fn mock_poll(
        AxumState(st): AxumState<Arc<MockApiState>>,
        AxumPath(_id): AxumPath<String>,
    ) -> Json<serde_json::Value> {
        let n = st.script.len();
        if n == 0 {
            return Json(
                serde_json::json!({"id": "int_mock", "status": "in_progress", "steps": []}),
            );
        }
        let i = st.polls.fetch_add(1, Ordering::SeqCst);
        Json(st.script[i.min(n - 1)].clone())
    }

    /// Cancel interaction
    async fn mock_cancel(
        AxumState(st): AxumState<Arc<MockApiState>>,
        AxumPath(_id): AxumPath<String>,
    ) -> Json<serde_json::Value> {
        st.cancels.fetch_add(1, Ordering::SeqCst);
        Json(serde_json::json!({"id": "int_mock", "status": "cancelled", "steps": []}))
    }

    /// Spawn mock API on random local port
    async fn spawn_mock_api(script: Vec<serde_json::Value>) -> (String, Arc<MockApiState>) {
        let state = Arc::new(MockApiState {
            script,
            polls: AtomicUsize::new(0),
            cancels: AtomicUsize::new(0),
        });
        let app = Router::new()
            .route("/v1beta/interactions", post(mock_create))
            .route(
                "/v1beta/interactions/{id}",
                get(mock_poll).post(mock_cancel),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock api");
        let addr = listener.local_addr().expect("mock api addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock api serve");
        });
        (format!("http://{addr}/v1beta/"), state)
    }

    /// Construct mock client pointing to local mock API
    fn mock_client(base_url: &str) -> Gemini {
        Gemini::with_model_and_base_url(
            "test-key",
            "models/gemini-omni-1.1-flash".to_string(),
            base_url.parse().expect("base url"),
        )
        .expect("mock client")
    }

    /// Happy path: create → in_progress → completed (inline base64) → returns clip
    #[tokio::test]
    async fn test_poll_loop_completes_against_mock_api() {
        let clip = b"mock video bytes for the poll-loop test";
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, clip);
        let completed = serde_json::json!({
            "id": "int_mock",
            "status": "completed",
            "steps": [
                { "type": "model_output", "content": [
                    { "type": "video", "data": b64, "mimeType": "video/mp4" }
                ]}
            ]
        });
        let (base, state) = spawn_mock_api(vec![
            serde_json::json!({"id": "int_mock", "status": "in_progress", "steps": []}),
            completed,
        ])
        .await;

        let videos = generate_with_client(
            &mock_client(&base),
            "test-key",
            "gemini-omni-1.1-flash",
            "a wave",
            Some(4),
            Some("9:16"),
            None,
            Duration::from_millis(5),
            Duration::from_secs(5),
            || false,
            None,
        )
        .await
        .expect("generate against mock api");

        assert_eq!(videos.len(), 1);
        assert_eq!(videos[0].data, clip.to_vec(), "inline base64 decoded");
        assert_eq!(videos[0].mime_type, "video/mp4");
        assert_eq!(
            state.polls.load(Ordering::SeqCst),
            2,
            "one in_progress poll + one completed poll"
        );
        assert_eq!(state.cancels.load(Ordering::SeqCst), 0, "no cancel call");
    }

    /// Cancellation: `is_cancelled()` = true immediately → cancels on server
    #[tokio::test]
    async fn test_poll_loop_cancellation_calls_server_cancel() {
        let (base, state) = spawn_mock_api(vec![]).await;
        let err = generate_with_client(
            &mock_client(&base),
            "test-key",
            "gemini-omni-1.1-flash",
            "a wave",
            None,
            None,
            None,
            Duration::from_millis(5),
            Duration::from_secs(5),
            || true,
            None,
        )
        .await
        .expect_err("cancelled before first poll");
        assert!(err.to_string().contains("cancelled by caller"), "{err}");
        assert_eq!(
            state.cancels.load(Ordering::SeqCst),
            1,
            "cancel_interaction sent server-side"
        );
        assert_eq!(state.polls.load(Ordering::SeqCst), 0, "never polled");
    }

    /// Terminal status other than completed (e.g. failed) → reports error
    #[tokio::test]
    async fn test_poll_loop_failed_status_reports_error() {
        let (base, state) = spawn_mock_api(vec![serde_json::json!({
            "id": "int_mock", "status": "failed", "steps": []
        })])
        .await;
        let err = generate_with_client(
            &mock_client(&base),
            "test-key",
            "gemini-omni-1.1-flash",
            "a wave",
            None,
            None,
            None,
            Duration::from_millis(5),
            Duration::from_secs(5),
            || false,
            None,
        )
        .await
        .expect_err("failed status must not succeed");
        assert!(
            err.to_string().contains("ended with status Failed"),
            "{err}"
        );
        assert_eq!(
            state.polls.load(Ordering::SeqCst),
            1,
            "stopped at first poll"
        );
    }

    /// Timeout: deadline = 0 → bails before first poll
    #[tokio::test]
    async fn test_poll_loop_timeout_fires_before_first_poll() {
        let (base, state) = spawn_mock_api(vec![]).await;
        let err = generate_with_client(
            &mock_client(&base),
            "test-key",
            "gemini-omni-1.1-flash",
            "a wave",
            None,
            None,
            None,
            Duration::from_millis(1),
            Duration::ZERO,
            || false,
            None,
        )
        .await
        .expect_err("zero timeout");
        assert!(err.to_string().contains("timed out"), "{err}");
        assert_eq!(state.polls.load(Ordering::SeqCst), 0, "never polled");
    }

    /// Empty prompt rejected at public `generate` before creating client
    #[tokio::test]
    async fn test_generate_rejects_empty_prompt() {
        let err = generate(
            "unused-key",
            "gemini-omni-1.1-flash",
            "   ",
            None,
            None,
            None,
            Duration::from_millis(1),
            Duration::from_secs(1),
            || false,
            None,
        )
        .await
        .expect_err("empty prompt");
        assert!(err.to_string().contains("prompt is empty"), "{err}");
    }
}