//! `manifest.json` ต่อการเรียก tool หนึ่งครั้ง — กลไก traceability ตาม
//! wiki/multi-model-generation.md §4.3 (เก็บ intermediate artifacts ทุกชิ้น
//! เพื่อ debug และ selective regeneration)
//!
//! น้ำหนักเบา: serialize/deserialize ล้วน ไม่มี I/O นอก `save()`

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// ไฟล์ output หนึ่งไฟล์ที่เครื่องมือเขียนลง run dir
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputEntry {
    /// ชื่อไฟล์ relative กับ run dir เช่น `scene-1.jpg` (เก็บชื่อสั้น ๆ ไม่ใช่ path เต็ม)
    pub path: String,
    /// MIME type ที่ API คืนมาจริง (เช่น `image/jpeg`, `audio/wav`) — ห้ามเดาเอง
    pub mime_type: String,
    /// ขนาดไฟล์เป็น byte
    pub bytes: u64,
    /// ประมาณความยาวเสียง (วินาที) — มีเฉพาะ output ที่เป็นเสียง (WAV header parse ได้)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
}

/// manifest ของการเรียก tool media หนึ่งครั้ง
///
/// เขียนลง `<run_dir>/manifest.json` ผ่าน [`Manifest::save`] — เป็น output
/// อย่างเดียว จึง derive เฉพาะ `Serialize` (ไม่มีฝั่งไหนอ่านกลับเป็น struct)
#[derive(Debug, Clone, Serialize)]
pub struct Manifest {
    /// เวอร์ชันของ schema นี้ — เพิ่มเมื่อเปลี่ยนรูปแบบในอนาคต
    pub schema_version: u32,
    /// ชื่อเครื่องมือที่เรียก เช่น `generate_image`
    pub tool: String,
    /// model ID ที่ใช้จริง (จาก `media.*_model` ใน nami.json)
    pub model: String,
    /// prompt / text ต้นฉบับที่ส่งให้ model
    pub prompt: String,
    /// พารามิเตอร์เฉพาะเครื่องมือ (aspect_ratio, voice_id ฯลฯ) เก็บเป็น JSON ดิบ
    pub params: serde_json::Value,
    /// ไฟล์ที่เขียนลง run dir ทั้งหมดของการเรียกนี้
    pub outputs: Vec<OutputEntry>,
    /// "running" ตอนเริ่ม, "ok" หรือ "error" หลัง [`Manifest::finish_ok`] / [`Manifest::finish_err`]
    pub status: String,
    /// ข้อความ error เมื่อ `status == "error"`
    pub error: Option<String>,
    /// เวลาเริ่ม (RFC3339, UTC)
    pub started_at: String,
    /// เวลาเสร็จ (RFC3339, UTC) — `None` ขณะยัง running
    pub finished_at: Option<String>,
    /// ระยะเวลาทำการจริง (ms) — คำนวณจาก `started` ตอน finish
    pub duration_ms: Option<u64>,
    /// นาฬิกาจริงตอน start — ไม่ถูก serialize (ใช้คำนวณ `duration_ms` เท่านั้น)
    #[serde(skip)]
    started: Instant,
}

impl Manifest {
    /// เริ่ม manifest ใหม่ (status = "running", จับเวลาไว้แล้ว)
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

    /// บันทึก output หนึ่งไฟล์ (เรียกได้หลายครั้ง — หลายไฟล์ต่อการเรียกเดียว)
    ///
    /// คืน `&mut` ของ entry ที่เพิ่ม เพื่อให้ตั้ง field เฉพาะชนิดได้ เช่น
    /// `duration_seconds` ของไฟล์เสียง
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

    /// ทำเครื่องหมายว่าสำเร็จ (status = "ok") และจับเวลาจบ
    pub fn finish_ok(&mut self) {
        self.finish("ok", None);
    }

    /// ทำเครื่องหมายว่าล้มเหลว (status = "error") พร้อมข้อความ
    pub fn finish_err(&mut self, error: &str) {
        self.finish("error", Some(error.to_string()));
    }

    fn finish(&mut self, status: &str, error: Option<String>) {
        self.status = status.to_string();
        self.error = error;
        self.finished_at = Some(chrono::Utc::now().to_rfc3339());
        self.duration_ms = Some(self.started.elapsed().as_millis() as u64);
    }

    /// เขียนลง `<run_dir>/manifest.json` (pretty JSON)
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

    /// start → add_output → finish_ok → save แล้วอ่านไฟล์กลับเป็น JSON: field ครบ,
    /// status "ok", outputs ถูกต้อง, duration ถูกจับไว้
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
        // add_output คืน &mut entry — ตั้ง duration ของไฟล์เสียงได้
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

    /// finish_err ต้องเซ็ต status "error" + error message และยัง save ได้
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

    /// status เริ่มต้นต้องเป็น "running" และ output ยังว่าง
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
