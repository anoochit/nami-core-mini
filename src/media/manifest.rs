//! `manifest.json` per tool invocation — traceability mechanism per
//! wiki/multi-model-generation.md §4.3 (stores all intermediate artifacts for
//! debugging and selective regeneration)
//!
//! Lightweight: pure serialization/deserialization without I/O except `save()`

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Single output file entry written by a tool to run dir
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputEntry {
    /// Relative filename within run dir, e.g. `scene-1.jpg` (stores short filename, not full path)
    pub path: String,
    /// Actual MIME type returned by API (e.g. `image/jpeg`, `audio/wav`) — do not guess
    pub mime_type: String,
    /// File size in bytes
    pub bytes: u64,
    /// Estimated audio duration (seconds) — only present for audio outputs (parsed from WAV header)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
}

/// Manifest for a single media tool execution
///
/// Saved to `<run_dir>/manifest.json` via [`Manifest::save`] — output-only struct,
/// so it derives `Serialize` only (never read back into a struct).
#[derive(Debug, Clone, Serialize)]
pub struct Manifest {
    /// Schema version — incremented for future format changes
    pub schema_version: u32,
    /// Tool name invoked, e.g. `generate_image`
    pub tool: String,
    /// Model ID actually used (from `media.*_model` in nami.json)
    pub model: String,
    /// Original prompt / text submitted to model
    pub prompt: String,
    /// Tool-specific parameters (aspect_ratio, voice_id, etc.) stored as raw JSON
    pub params: serde_json::Value,
    /// All output files written to run dir during this invocation
    pub outputs: Vec<OutputEntry>,
    /// "running" initially, "ok" or "error" after [`Manifest::finish_ok`] / [`Manifest::finish_err`]
    pub status: String,
    /// Error message when `status == "error"`
    pub error: Option<String>,
    /// Start timestamp (RFC3339, UTC)
    pub started_at: String,
    /// Finish timestamp (RFC3339, UTC) — `None` while running
    pub finished_at: Option<String>,
    /// Execution duration (ms) — calculated from `started` at finish
    pub duration_ms: Option<u64>,
    /// Monotonic instant at start — skipped from serialization (used only to calculate `duration_ms`)
    #[serde(skip)]
    started: Instant,
}

impl Manifest {
    /// Start a new manifest (status = "running", start timer)
    pub fn start(tool: &str, model: &str, prompt: &str, params: serde_json::Value) -> Self {
        Self {
            schema_version: 1,
            tool: tool.to_string(),
            model: model.to_string(),
            prompt: prompt.to_string(),
            params,
            outputs: Vec::new(),
            status: "running".to_string(),
            error: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            duration_ms: None,
            started: Instant::now(),
        }
    }

    /// Record an output file (can be called multiple times for multi-file outputs)
    ///
    /// Returns `&mut` of the newly added entry to allow setting specific fields, such as
    /// `duration_seconds` for audio files.
    pub fn add_output(
        &mut self,
        path: impl Into<String>,
        mime_type: impl Into<String>,
        bytes: u64,
    ) -> &mut OutputEntry {
        self.outputs.push(OutputEntry {
            path: path.into(),
            mime_type: mime_type.into(),
            bytes,
            duration_seconds: None,
        });
        self.outputs.last_mut().expect("entry just pushed")
    }

    /// Mark manifest as successful (status = "ok") and stop timer
    pub fn finish_ok(&mut self) {
        self.finish("ok", None);
    }

    /// Mark manifest as failed (status = "error") with error message
    pub fn finish_err(&mut self, error: &str) {
        self.finish("error", Some(error.to_string()));
    }

    fn finish(&mut self, status: &str, error: Option<String>) {
        self.status = status.to_string();
        self.error = error;
        self.finished_at = Some(chrono::Utc::now().to_rfc3339());
        self.duration_ms = Some(self.started.elapsed().as_millis() as u64);
    }

    /// Save manifest to `<run_dir>/manifest.json` (pretty JSON)
    pub fn save(&self, run_dir: &Path) -> anyhow::Result<PathBuf> {
        let path = run_dir.join("manifest.json");
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| anyhow::anyhow!("cannot serialize manifest: {e}"))?;
        std::fs::write(&path, json)
            .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display()))?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// start → add_output → finish_ok → save and read back JSON: check fields,
    /// status "ok", correct outputs, duration recorded
    #[test]
    fn test_manifest_round_trip() {
        let dir = std::env::temp_dir().join(format!("nami-manifest-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut m = Manifest::start(
            "generate_image",
            "gemini-3.1-flash-image",
            "a wave",
            serde_json::json!({ "aspect_ratio": "16:9" }),
        );
        m.add_output("wave.jpg", "image/jpeg", 668_152);
        // add_output returns &mut entry — allow setting duration on audio files
        m.add_output("speech-1.wav", "audio/wav", 203_876)
            .duration_seconds = Some(4.25);
        m.finish_ok();
        let saved = m.save(&dir).unwrap();
        assert_eq!(saved, dir.join("manifest.json"));

        let loaded: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
        assert_eq!(loaded["schema_version"], 1);
        assert_eq!(loaded["tool"], "generate_image");
        assert_eq!(loaded["model"], "gemini-3.1-flash-image");
        assert_eq!(loaded["prompt"], "a wave");
        assert_eq!(loaded["params"]["aspect_ratio"], "16:9");
        assert_eq!(loaded["status"], "ok");
        assert!(loaded["error"].is_null());
        assert_eq!(loaded["outputs"].as_array().unwrap().len(), 2);
        assert_eq!(loaded["outputs"][0]["path"], "wave.jpg");
        assert_eq!(loaded["outputs"][0]["mime_type"], "image/jpeg");
        assert_eq!(loaded["outputs"][0]["bytes"], 668_152);
        assert!(
            loaded["outputs"][0]["duration_seconds"].is_null(),
            "image output has no duration (field skipped)"
        );
        assert_eq!(loaded["outputs"][1]["duration_seconds"], 4.25);
        assert!(loaded["finished_at"].is_string());
        assert!(loaded["duration_ms"].is_u64());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// finish_err must set status "error" + error message and remain savable
    #[test]
    fn test_manifest_finish_err() {
        let mut m = Manifest::start(
            "generate_tts",
            "gemini-3.8-flash-tts",
            "hi",
            serde_json::json!({}),
        );
        m.finish_err("quota exceeded");
        assert_eq!(m.status, "error");
        assert_eq!(m.error.as_deref(), Some("quota exceeded"));
        assert!(m.duration_ms.is_some());
    }

    /// Initial status must be "running" with empty outputs
    #[test]
    fn test_manifest_starts_running() {
        let m = Manifest::start(
            "generate_video",
            "gemini-omni-1.1-flash",
            "p",
            serde_json::Value::Null,
        );
        assert_eq!(m.status, "running");
        assert!(m.outputs.is_empty());
        assert!(m.finished_at.is_none());
        assert!(m.duration_ms.is_none());
    }
}