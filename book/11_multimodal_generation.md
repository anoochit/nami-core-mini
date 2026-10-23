# Chapter 11: Multimodal Generation

Text is cheap to reason about and useless as a deliverable. This chapter is about the part
of me that turns a sentence into a file you can open — an image, a voiceover, a music bed, a
short clip, and finally a finished MP4 with all of it mixed together.

There is one rule that governs every line of this feature, and everything else follows from
it:

**Media travels by path, never by bytes.**

A generated asset is written to disk inside the tool, and the tool returns a path plus
metadata. Base64 never enters the model's context. A single 1080p frame is hundreds of
kilobytes of base64; a five-scene pipeline would blow the context window on the first batch.
Worse, it would never leave — the bytes would sit in the conversation for every subsequent
turn. So the tools are built so that this is not a rule you have to remember. It is the only
thing they can do.

## 1. The Five Tools

| Tool | Output | Notes |
|------|--------|-------|
| `generate_image` | `image-N.jpg` | Aspect-ratio whitelist of 10 values |
| `generate_tts` | `speech-N.wav` | `.mp3` on request, via local ffmpeg |
| `generate_music` | `music-1.mp3` | Fixed ~30 s clip, Lyria |
| `generate_video` | `video-N.mp4` | 4–8 s, minutes to render, long-running |
| `assemble_audio_video` | `assembled-1.mp4` | Local ffmpeg; narration + music onto video |

They are created only when media is switched on:

```rust
// src/tools/media.rs
pub fn create_media_toolset(config: &NamiConfig) -> Vec<Arc<dyn Tool>> {
    if !config.media.enabled {
        return vec![];
    }
    vec![
        create_image_tool(config.media.clone()),
        create_tts_tool(config.media.clone()),
        create_music_tool(config.media.clone()),
        create_video_tool(...),
        create_assemble_tool(...),
    ]
}
```

Note the asymmetry with everything else in this book: media tools attach to **sub-agents**
via the `"media"` value in `AgentTool`, not to root. The root agent never gets them. The
shipped `nami.json` has one specialist wired for it, `media`, whose `tools` array contains
`"media"` and nothing else.

## 2. Why I Call the Wire API Directly

The obvious implementation would use the `Llm` abstraction, like the chat model does. It
does not work, and the reason is worth stating precisely:

> `response_modalities` / `speech_config` are dropped when the config is converted (see
> `adk-gemini`'s `client.rs`), and `GenerationConfig` has no `imageConfig` for aspect
> ratio. `GeminiClient` does not expose raw `execute` externally.

So image, TTS, and music build their JSON body by hand and POST it themselves:

```rust
// src/media/mod.rs
pub(crate) async fn post_generate_content(
    api_key: &str, model: &str, body: &serde_json::Value, timeout: Duration,
) -> anyhow::Result<adk_gemini::GenerationResponse> {
    let url = format!("{GEMINI_BASE_URL}/models/{model}:generateContent");
    ...
    .header("x-goog-api-key", api_key)
```

The compromise is deliberate: hand-build the *request*, but let `adk-gemini`'s typed
`GenerationResponse` parse the *response*. Request shapes are what the SDK drops; response
shapes are what it tests. Splitting there buys raw control without giving up typed parsing.

An image request in full:

```rust
// src/media/image.rs
let mut generation_config = serde_json::json!({ "responseModalities": ["TEXT", "IMAGE"] });
if let Some(ar) = aspect_ratio {
    generation_config["imageConfig"] = serde_json::json!({ "aspectRatio": ar });
}
```

## 3. Video Is a Different Animal

This is the part that cost real debugging time, so it is documented rather than summarised.

Veo is gone. `veo-3.1-generate-preview` still appears in the model list, and calling it
returns 404 "model is no longer available". The one video model that works,
`gemini-omni-1.1-flash`, accepts **only** the Interactions API — a plain `generateContent`
call returns 400 "This model only supports Interactions API".

Which means video cannot share `post_generate_content` at all. It goes through the typed
Interactions client in a three-step dance:

```rust
// src/media/video.rs
// 1. create interaction (background) → interaction id immediately
let created = client.send_interaction(request).await?;
// 2. poll get_interaction until status == Completed
// 3. pull Content::Video from the ModelOutput step
```

`background: Some(true)` is what lets the call return immediately instead of blocking on a
server render that takes minutes. The clip arrives either as inline base64 or as a `uri`, and
both paths are handled.

Two consequences you will feel:

- **Duration and aspect ratio are not parameters.** The model has no config field for them,
  so they are appended to the prompt as instructions:

  ```rust
  out.push_str(&format!("\n\nThe clip must be exactly {d} seconds long."));
  ```

  The whitelist (`4`, `6`, `8`; `16:9`, `9:16`) exists to give the model fast feedback
  before an API call, not because the API enforces it. Ask for 5 seconds and you will be
  told no; ask for 6 and you may still get something that is not exactly 6.
- **The tool is long-running.** `.with_long_running(true)` marks it so, and Chapter 15's
  timeout adjustment exists because of it.

## 4. Run Directories and `manifest.json`

Every tool call gets its own directory:

```
media/run-20261023-091405-a3f19c2d/
├── image-1.jpg
├── speech-1.wav
└── manifest.json
```

The name is `run-<yyyymmdd-hhmmss>-<8 hex>`, and collisions error rather than overwrite.
Inside sits a `manifest.json` recording what was asked for and what came back:

```rust
// src/media/manifest.rs
pub struct Manifest {
    pub schema_version: u32,
    pub tool: String,        // "generate_image"
    pub model: String,       // the model actually used
    pub prompt: String,
    pub params: serde_json::Value,
    pub outputs: Vec<OutputEntry>,
    pub status: String,      // "running" → "ok" | "error"
    pub error: Option<String>,
    ...
}
```

The manifest is written on **failure too** — `finish_err` then `save`. That is the point of
it. When a scene comes out wrong you get the exact prompt, the exact model, and the error
message, and you regenerate that one scene without touching the other four. Selective
regeneration is the whole reason this is not just a folder of files.

`OutputEntry.mime_type` is always the MIME the API actually returned — the code refuses to
guess it, and maps it to a file extension only after the fact.

## 5. Concurrency, and Why It Is Fast

Media calls are slow and independent, so all five tools are marked concurrency-safe:

```rust
// src/tools/media.rs
// Each media tool call writes its own file — safe for parallel execution
.with_concurrency_safe(true)
```

That claim is true precisely *because* every call writes to its own run directory. Nothing
shares a filename, so there is nothing to race on.

The sub-agent that carries media tools also switches dispatch strategy and raises its
timeout:

```rust
// src/agent.rs
if has_media_tools {
    sub_builder = sub_builder
        .tool_execution_strategy(adk_rust::ToolExecutionStrategy::Parallel)
        .tool_timeout(std::time::Duration::from_secs(config.media.timeout_secs + 60));
}
```

The timeout line is not cosmetic. The runner's default tool timeout is 300 s; `media.timeout_secs`
defaults to 600 s, and the runner's timeout is evaluated *before* the tool's own — so without
that `+ 60`, a video render is killed mid-flight at the five-minute mark.

The payoff is in the skill: the model emits one response containing `generate_image` and
`generate_tts` for every scene at once, and those calls genuinely run concurrently.

## 6. Watching It Work

Video renders stream progress back through the tool context, about every 30 seconds:

```rust
emit_progress(progress, &format!("still rendering — {}s elapsed", started.elapsed().as_secs()))
```

The CLI flattens those into live lines in the `tool:` section, and deliberately excludes
them from the turn stats — a progress message is not model output:

```
tool:
generate_video({"prompt":"A calm ocean wave at golden hour"})
… video requested — the server is rendering now (this can take minutes)...
… still rendering — 30s elapsed
✓ generate_video
  media/run-20261023-091405-a3f19c2d/video-1.mp4 (1.2 MB · 8.0s)
```

Cancellation is checked on every poll and is not local-only — it best-effort cancels the
interaction server-side too, so you do not pay for a render nobody is waiting for.

## 7. The Guardrails

Three of the security habits from Chapter 18 reappear here, and one is new.

```rust
// src/media/mod.rs
pub(crate) fn host_is_google(url: &str) -> bool {
    ...is_some_and(|h| h.ends_with("googleapis.com") || h.ends_with("googleusercontent.com"))
}
```

When a video comes back as a `uri` rather than inline data, it has to be downloaded — and
downloading requires attaching the API key. That key is attached **only** when the host is a
Google domain. `https://evil.example.com/steal` gets a plain request; so does
`https://generativelanguage.googleapis.com.download/storage/x.mp4`, which is the lookalike
that a `contains("googleapis.com")` check would have let straight through. There is a test
for exactly that case.

Beyond it: `require_api_key` rejects the `your_gemini_api_key_here` placeholder, so a fresh
clone fails with a message instead of a confusing 401. `WorkspaceGuard` validates
`reference_image_path` before reading it and every input path to `assemble_audio_video`.
Error bodies are truncated to 300 characters, on a char boundary, so a Thai or emoji error
message cannot panic the truncator.

## 8. Assembly

`assemble_audio_video` is the only tool that runs something local rather than calling an
API. It shells out to ffmpeg, and its policy is stated in one line of argument building:

```rust
// src/media/assemble.rs
// - Every case: `-c:v copy` (video is not re-encoded) + `apad` + `-shortest`
//   = video is the master duration (longer audio is cut, shorter is padded)
```

**Video duration is master.** Narration shorter than the clip is padded with silence;
longer is cut at the end. That is the right default for this pipeline, because the clip came
from a model with a fixed length and the narration came from a sentence you wrote.

The video stream is never re-encoded (`-c:v copy`), which keeps assembly fast and lossless,
and `-movflags +faststart` puts the index at the front so the MP4 plays while it downloads.
Multiple audio sources go through `filter_complex` + `amix`; a single source is mapped
directly with no filter at all. `music_volume` defaults to 0.3 when mixed under narration and
1.0 when music is the only audio — a small default that stops background music from
swamping a voiceover.

Requires ffmpeg on `PATH`. Both ffmpeg-dependent features check for it up front and return
a clear error rather than a confusing one from a failed spawn.

## 9. Configuration

```json
"media": { "enabled": true }
```

That is all the shipped config needs — every other field has a default:

| Field | Default |
|-------|---------|
| `enabled` | `false` — media is **opt-in** |
| `image_model` | `gemini-3.1-flash-image` |
| `tts_model` | `gemini-3.8-flash-tts` |
| `tts_voice` | `Kore` |
| `video_model` | `gemini-omni-1.1-flash` |
| `music_model` | `lyria-3-clip-preview` |
| `output_dir` | `./media` |
| `poll_interval_secs` | `5` |
| `timeout_secs` | `600` |

Model IDs live in config and never in code, for one reason: Google retires them without
notice. Every default above was confirmed with a real API call, and the ones that are wrong
today were wrong in a way only a live call could reveal — that is how the Veo situation was
discovered.

Opt-in matters. If you declare `"media"` in a sub-agent's tools while `media.enabled` is
false, that agent starts with no tools and a warning naming it. Media costs money per call;
it should never be one forgotten config key away from running.

## 10. The Skill That Ties It Together

Tools give capability; `.skills/multimodal-pipeline/SKILL.md` gives the procedure — five
steps: decompose the script into 3–6 scenes, batch-generate every asset in one response, add
motion with `reference_image_path`, assemble with ffmpeg, report paths.

Two rules from it are worth quoting because they encode everything above:

> Never put media base64/bytes into the conversation context — reference files by path only.

> If a tool returns an error, fix the prompt and re-run only the failed scene.

That second one is why the manifest exists.

## 11. Honest Limits

- **No duration control on music.** Lyria returns ~30 s and `durationSeconds` is rejected
  with a 400. Style is tuned through the prompt; length is not tunable at all.
- **Aspect ratio and duration on video are hints.** Prompted, not enforced.
- **TTS emotion is a prefix.** `emotion` becomes `"Deliver the following text in a {} tone."`
  prepended to your text — there is no dedicated parameter.
- **MP3 conversion needs ffmpeg**, on `PATH`, at call time.
- **`src/media/mod.rs` still carries `#![allow(dead_code)]`** with a comment saying the API
  has no callers until later phases. That comment is now stale — the tools call it — but the
  attribute is still there.
- **One voice per `generate_tts` call.** Multi-speaker narration means calling it per scene
  with different `voice_id` values and concatenating, which the pipeline has to do manually.

Next: how I decide to hand work to someone else.
