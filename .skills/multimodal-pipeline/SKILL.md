---
name: multimodal-pipeline
description: Generate multimedia content (image sets, voiceover audio, music tracks, short video clips, assembled final videos) as a pipeline - decompose a script into scenes, batch-generate assets in parallel, add motion with reference images, mix narration and music onto video with ffmpeg, and report file paths. Use when the user asks for videos, image sets, voiceovers, music, slideshows, or story-driven media.
tags: [media, video, image, tts, music, generation, pipeline, assembly]
---

# Multimodal Pipeline

How to produce multimedia content (images, narration audio, background music,
video) from a single topic or story using the tools `generate_image`,
`generate_tts`, `generate_music`, `generate_video`, `assemble_audio_video`
(requires `media.enabled` in nami.json and `media` in the sub-agent's `tools`).

## Steps

### 1. Script decomposition

- Split the story into 3-6 short, focused scenes. Each scene has:
  - **visual prompt**: describe subject + action + lighting/mood + camera angle
    clearly; say "static camera" when no camera movement is wanted (steadier
    results)
  - **narration**: one short sentence of voiceover text
- English prompts work better for the image/video models

### 2. Parallel asset generation

- Call `generate_image` and `generate_tts` for **every scene in a single
  response** (include `generate_music` for background music - another tool that
  runs in parallel). Media tools are marked concurrency-safe, and the sub-agent
  uses parallel tool execution, so the calls really do run concurrently.
- Background music: `generate_music` returns a fixed ~30 second clip (no
  duration parameter) - tune the style through `style`/prompt; use it to open
  the video or bridge scenes
- Tools return **path + metadata only** - keep the paths for the next step and
  never request the data again as base64/bytes

### 3. Motion synthesis

- For scenes that need motion, call `generate_video` with
  `reference_image_path` = the path of that scene's image (it becomes the first
  frame of the clip)
- Clips are 4-8 seconds - `duration_seconds`/`aspect_ratio` are prompt hints,
  not hard model parameters
- Each clip takes minutes to render - call several clips in one response so
  they run in parallel (or skip this step when unnecessary)

### 4. Assembly

- Combine the pieces into a finished clip with `assemble_audio_video`:
  - `video_path`: the clip to build on (a `generate_video` output, or any MP4)
  - `audio_path`: narration to overlay (`generate_tts` output)
  - `music_path`: background music to mix underneath (`generate_music` output)
  - `music_volume`: 0.0-1.0; keep music quiet (0.2-0.4) when narration is present
  - `keep_video_audio`: true to retain the clip's own audio in the mix
- The video's duration is the master: narration/music shorter than the clip is
  padded with silence, longer is cut at the end of the video
- Requires ffmpeg installed locally; the assembled MP4 lands in the run dir
- For scene-by-scene narration with different voices, call `generate_tts` per
  scene with a different `voice_id`, then concatenate the clips or assemble
  scene-by-scene

### 5. Deliverables

- Report the path of every file + metadata (size, duration, etc.) to the caller
- All files live under `media/<run-id>/` with a `manifest.json` for traceability
- To regenerate a single scene: call the tool again for that scene only - no
  need to rebuild the others

## Constraints

- Never put media base64/bytes into the conversation context - reference files
  by path only
- If a tool returns an error, fix the prompt and re-run only the failed scene
- Voiceover: `generate_tts` takes `emotion` as an English prefix (e.g.
  "Cheerful and fast-paced:"), `voice_id` when specified, and `output_format`
  ("wav" default, "mp3" converted locally - needs ffmpeg)
