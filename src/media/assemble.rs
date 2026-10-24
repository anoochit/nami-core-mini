//! Phase 7: การประกอบไฟล์ media ด้วย ffmpeg (local — ไม่มี API call)
//!
//! ทำสองอย่าง:
//! 1. **assembly** — ประกอบวิดีโอ + narration (+ music) เป็น MP4 สุดท้าย
//!    (video duration เป็น master: เสียงถูก pad/cut ให้ตรงด้วย `apad` + `-shortest`)
//! 2. **conversion** — แปลง WAV ของ TTS เป็น mp3 (libmp3lame) ตาม
//!    `output_format` ของ `generate_tts`
//!
//! ffmpeg ต้องอยู่ใน PATH ของเครื่อง (`ffmpeg -version` ผ่าน = ใช้ได้) —
//! ถ้าไม่มี tool จะคืน error บอกให้ติดตั้ง ไม่ทำให้整个 pipeline พัง

use anyhow::{Context, bail};
use std::path::Path;
use std::time::{Duration, Instant};

/// ตรวจว่า ffmpeg ใน PATH ใช้ได้ (`ffmpeg -version` exit 0)
pub async fn ensure_ffmpeg() -> anyhow::Result<()> {
    let ffmpeg_cmd = crate::mcp::resolve_executable("ffmpeg");
    let ok = tokio::process::Command::new(&ffmpeg_cmd)
        .arg("-version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        bail!(
            "ffmpeg not found in PATH — install ffmpeg (https://ffmpeg.org) to \
             assemble or convert media"
        );
    }
    Ok(())
}

/// ข้อมูล inputs สำหรับ build ffmpeg args (assembly)
pub struct AssembleInputs<'a> {
    /// วิดีโอต้นฉบับ — จะเป็น input index 0 และ duration master ของ output
    pub video: &'a Path,
    /// narration/voiceover (input index 1 ถ้ามี)
    pub narration: Option<&'a Path>,
    /// background music (input index 2 ถ้ามี narration ด้วย, ไม่งั้น 1)
    pub music: Option<&'a Path>,
    /// ระดับเสียง music 0.0–1.0 (ค่า default เมื่อ mix กับ source อื่น = 0.3)
    pub music_volume: f32,
    /// เก็บเสียงเดิมของวิดีโอ (`0:a`) ไว้ใน mix ด้วยหรือไม่
    pub keep_video_audio: bool,
    /// output mp4 path
    pub output: &'a Path,
}

/// สร้าง argv ของ ffmpeg จาก inputs (pure function — ทดสอบได้ไม่ต้องรัน ffmpeg)
///
/// กลยุทธ์:
/// - audio source เดียว → map ตรง ๆ ไม่ใช้ filter (`-af` ใส่ volume/apad ได้)
/// - หลาย source → `filter_complex` + `amix` แล้ว map `[a]`
/// - ทุกกรณี: `-c:v copy` (video ไม่ re-encode) + `apad` + `-shortest`
///   = วิดีโอเป็น master duration (เสียงยาวกว่าถูกตัด, สั้นกว่าถูก pad)
/// - `-movflags +faststart` สำหรับการเล่น MP4 บนเว็บ
pub fn build_ffmpeg_args(inputs: &AssembleInputs<'_>) -> Vec<String> {
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-y".into()];
    args.push("-i".into());
    args.push(inputs.video.display().to_string());
    if let Some(n) = inputs.narration {
        args.push("-i".into());
        args.push(n.display().to_string());
    }
    if let Some(m) = inputs.music {
        args.push("-i".into());
        args.push(m.display().to_string());
    }

    // ลำดับ input: video=0, narration=1 (ถ้ามี), music=narration?2:1
    let music_idx = if inputs.narration.is_some() { 2 } else { 1 };

    // จำนวน audio sources ที่จะ mix
    let sources_count = usize::from(inputs.keep_video_audio)
        + usize::from(inputs.narration.is_some())
        + usize::from(inputs.music.is_some());

    match sources_count {
        0 => {
            // ไม่มีเสียงเลย — map เฉพาะวิดีโอ
            args.push("-map".into());
            args.push("0:v".into());
        }
        1 if inputs.keep_video_audio => {
            // เสียงเดิมของวิดีโอ — `0:a?` เผื่อคลิปที่ไม่มี audio stream
            args.push("-map".into());
            args.push("0:v".into());
            args.push("-map".into());
            args.push("0:a?".into());
        }
        1 => {
            // narration หรือ music เดียว — map input ตรง ๆ ไม่ต้อง amix
            let idx = if inputs.music.is_some() { music_idx } else { 1 };
            args.push("-map".into());
            args.push("0:v".into());
            args.push("-map".into());
            args.push(format!("{idx}:a"));
        }
        _ => {
            // หลาย source — filter_complex + amix (labels ทุกตัวเป็น [..])
            let mut chain_labels: Vec<String> = Vec::new();
            if inputs.keep_video_audio {
                chain_labels.push("[0:a]".into());
            }
            if inputs.narration.is_some() {
                chain_labels.push("[1:a]".into());
            }
            if inputs.music.is_some() {
                chain_labels.push("[music]".into());
            }
            let mut chain = String::new();
            if inputs.music.is_some() {
                chain.push_str(&format!(
                    "[{music_idx}:a]volume={}[music];",
                    inputs.music_volume
                ));
            }
            chain.push_str(&format!(
                "{}amix=inputs={sources_count}:duration=longest,apad[a]",
                chain_labels.concat()
            ));
            args.push("-filter_complex".into());
            args.push(chain);
            args.push("-map".into());
            args.push("0:v".into());
            args.push("-map".into());
            args.push("[a]".into());
        }
    }

    // เสียงเดียว (ไม่มี filter_complex) ต้อง apply volume/apad ผ่าน -af
    if sources_count <= 1
        && sources_count > 0
        && (inputs.music.is_some() || inputs.narration.is_some())
    {
        let mut af: Vec<String> = Vec::new();
        if inputs.music.is_some() {
            af.push(format!("volume={}", inputs.music_volume));
        }
        af.push("apad".into());
        args.push("-af".into());
        args.push(af.join(","));
    }

    args.push("-c:v".into());
    args.push("copy".into());
    args.push("-c:a".into());
    args.push("aac".into());
    args.push("-shortest".into()); // จบเมื่อวิดีโอจบ (master)
    args.push("-movflags".into());
    args.push("+faststart".into());
    args.push(inputs.output.display().to_string());
    args
}

/// สร้าง argv สำหรับแปลงไฟล์เสียงเป็น mp3 (libmp3lame quality 2 ≈ 190 kbps)
pub fn build_mp3_args(input: &Path, output: &Path) -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-y".into(),
        "-i".into(),
        input.display().to_string(),
        "-codec:a".into(),
        "libmp3lame".into(),
        "-q:a".into(),
        "2".into(),
        output.display().to_string(),
    ]
}

/// รัน ffmpeg ด้วย args ที่เตรียมไว้ — poll ทุก 300ms เพื่อเช็ค cancel + timeout
///
/// stderr ถูก drain ตลอด (pipe buffer เต็ม = ffmpeg ค้าง) และเก็บ 32KB แรก
/// ไว้เป็นบริบทตอน error
pub async fn run_ffmpeg(
    args: &[String],
    timeout: Duration,
    is_cancelled: impl Fn() -> bool,
) -> anyhow::Result<()> {
    use tokio::io::AsyncReadExt;

    let ffmpeg_cmd = crate::mcp::resolve_executable("ffmpeg");
    let mut child = tokio::process::Command::new(&ffmpeg_cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("cannot spawn ffmpeg")?;

    // drain stderr ต่อเนื่อง — คืนเป็น 32KB แรก ( buffer เต็มก็ทิ้งส่วนเกิน)
    let mut stderr = child.stderr.take().context("cannot take ffmpeg stderr")?;
    let stderr_task = tokio::spawn(async move {
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stderr.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if buf.len() < 32 * 1024 {
                        buf.extend_from_slice(&chunk[..n.min(32 * 1024 - buf.len())]);
                    }
                }
            }
        }
        buf
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        if is_cancelled() {
            let _ = child.kill().await;
            let tail = stderr_task.await.unwrap_or_default();
            bail!("assembly cancelled by caller{}", stderr_hint(&tail));
        }
        if let Some(status) = child.try_wait().context("cannot poll ffmpeg")? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill().await;
            let tail = stderr_task.await.unwrap_or_default();
            bail!(
                "ffmpeg timed out after {}s{}",
                timeout.as_secs(),
                stderr_hint(&tail)
            );
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    };

    let tail = stderr_task.await.unwrap_or_default();
    if !status.success() {
        bail!("ffmpeg exited with status {status}{}", stderr_hint(&tail));
    }
    Ok(())
}

/// แปลงไฟล์เสียง (WAV จาก TTS) เป็น mp3 ด้วย libmp3lame
pub async fn convert_to_mp3(input: &Path, output: &Path, timeout: Duration) -> anyhow::Result<()> {
    let args = build_mp3_args(input, output);
    run_ffmpeg(&args, timeout, || false).await
}

/// แนบท้าย stderr ของ ffmpeg (500 อักขรสุดท้าย) เป็นบริบทตอน error
///
/// ลบ ANSI escape sequence (สี/ cursor) และ control chars แล้วยุบ whitespace
/// เป็นบรรทัดเดียว
fn stderr_hint(tail: &[u8]) -> String {
    let text = String::from_utf8_lossy(tail);
    let mut cleaned = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // CSI sequence (ESC [ ... final) — ข้ามจนถึง final byte
            if chars.peek() == Some(&'[') {
                chars.next();
                for c2 in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c2) {
                        break;
                    }
                }
            }
        } else if !c.is_control() || c == '\n' {
            cleaned.push(c);
        }
    }
    let flat = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return String::new();
    }
    const MAX: usize = 500;
    if flat.chars().count() > MAX {
        let cut: String = flat.chars().take(MAX).collect();
        format!(": {cut}…")
    } else {
        format!(": {flat}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn args_str(args: &[String]) -> String {
        args.join(" ")
    }

    /// video + narration (ไม่มี music) → map ตรง ๆ + apad + shortest + copy
    #[test]
    fn test_build_args_narration_only() {
        let video = PathBuf::from("media/run-1/video-1.mp4");
        let narration = PathBuf::from("media/run-1/speech-1.wav");
        let out = PathBuf::from("media/run-1/assembled-1.mp4");
        let args = build_ffmpeg_args(&AssembleInputs {
            video: &video,
            narration: Some(&narration),
            music: None,
            music_volume: 0.3,
            keep_video_audio: false,
            output: &out,
        });
        let s = args_str(&args);
        assert!(s.contains("-i media/run-1/video-1.mp4"));
        assert!(s.contains("-i media/run-1/speech-1.wav"));
        assert!(s.contains("-map 0:v"));
        assert!(s.contains("-map 1:a"));
        assert!(s.contains("-af apad"));
        assert!(s.contains("-c:v copy"));
        assert!(s.contains("-c:a aac"));
        assert!(s.contains("-shortest"));
        assert!(s.contains("-movflags +faststart"));
        assert!(s.contains("media/run-1/assembled-1.mp4"));
        assert!(!s.contains("filter_complex"), "single source ไม่ใช้ amix");
    }

    /// narration + music → filter_complex amix + volume ของ music
    #[test]
    fn test_build_args_narration_and_music_uses_amix() {
        let video = PathBuf::from("v.mp4");
        let narration = PathBuf::from("n.wav");
        let music = PathBuf::from("m.mp3");
        let out = PathBuf::from("o.mp4");
        let args = build_ffmpeg_args(&AssembleInputs {
            video: &video,
            narration: Some(&narration),
            music: Some(&music),
            music_volume: 0.25,
            keep_video_audio: false,
            output: &out,
        });
        let s = args_str(&args);
        let fc = args
            .iter()
            .position(|a| a == "-filter_complex")
            .map(|i| args[i + 1].clone())
            .expect("มี filter_complex");
        // music (index 2) ผ่าน volume ก่อนแล้วค่อย amix กับ narration (index 1)
        assert!(fc.starts_with("[2:a]volume=0.25[music];[1:a][music]amix=inputs=2"));
        assert!(fc.contains("duration=longest"));
        assert!(fc.contains("apad"));
        assert!(fc.ends_with("[a]"));
        assert!(s.contains("-map [a]"));
        // ไม่ใช้ -af คู่กับ filter_complex
        assert!(!s.contains("-af "));
    }

    /// keep_video_audio + music → 3 inputs เข้า amix (0:a ด้วย)
    #[test]
    fn test_build_args_keep_video_audio_in_mix() {
        let video = PathBuf::from("v.mp4");
        let music = PathBuf::from("m.mp3");
        let out = PathBuf::from("o.mp4");
        let args = build_ffmpeg_args(&AssembleInputs {
            video: &video,
            narration: None,
            music: Some(&music),
            music_volume: 0.5,
            keep_video_audio: true,
            output: &out,
        });
        let fc = args
            .iter()
            .position(|a| a == "-filter_complex")
            .map(|i| args[i + 1].clone())
            .unwrap();
        // music อยู่ input index 1 (ไม่มี narration) — video audio อยู่ 0:a
        assert!(
            fc.starts_with("[1:a]volume=0.5[music];[0:a][music]amix=inputs=2"),
            "got: {fc}"
        );
    }

    /// music เท่านั้น (ไม่มี keep/narration) → source เดียว + volume + apad
    #[test]
    fn test_build_args_music_only_single_source() {
        let video = PathBuf::from("v.mp4");
        let music = PathBuf::from("m.mp3");
        let out = PathBuf::from("o.mp4");
        let args = build_ffmpeg_args(&AssembleInputs {
            video: &video,
            narration: None,
            music: Some(&music),
            music_volume: 1.0,
            keep_video_audio: false,
            output: &out,
        });
        let s = args_str(&args);
        assert!(s.contains("-map 1:a"));
        assert!(s.contains("-af volume=1,apad"));
        assert!(!s.contains("filter_complex"));
    }

    /// mp3 conversion args ใช้ libmp3lame
    #[test]
    fn test_build_mp3_args() {
        let args = build_mp3_args(Path::new("in.wav"), Path::new("out.mp3"));
        let s = args_str(&args);
        assert!(s.contains("-i in.wav"));
        assert!(s.contains("-codec:a libmp3lame"));
        assert!(s.contains("-q:a 2"));
        assert!(s.ends_with("out.mp3"));
    }

    /// stderr hint ตัด control chars + ยุบเป็นบรรทัดเดียว + ตัดที่ 500 อักขระ
    #[test]
    fn test_stderr_hint() {
        assert_eq!(stderr_hint(b""), "");
        assert_eq!(
            stderr_hint(b"line1\r\n\x1b[31merror\x1b[0m: bad\n"),
            ": line1 error: bad"
        );
        let long = vec![b'a'; 600];
        assert!(stderr_hint(&long).chars().count() < 520);
    }

    /// ถ้าเครื่องมี ffmpeg จริง — ประกอบไฟล์ fixture ที่สร้างด้วย ffmpeg เอง
    /// (testsrc + sine ไม่มีค่าใช้จ่าย API) แล้วตรวจ output เป็น MP4 ที่เล่นได้
    ///
    /// ข้ามเงียบ ๆ ถ้าไม่มี ffmpeg ใน PATH
    #[tokio::test]
    #[ignore] // ต้องมี ffmpeg ในเครื่อง — รันด้วย `cargo test -- --ignored`
    async fn test_assemble_live_with_ffmpeg_fixtures() {
        if ensure_ffmpeg().await.is_err() {
            eprintln!("skip: ffmpeg not in PATH");
            return;
        }
        let dir = std::env::temp_dir().join(format!("nami-assemble-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("video.mp4");
        let wav = dir.join("speech.wav");
        let music = dir.join("music.mp3");
        let out = dir.join("assembled.mp4");
        let short = Duration::from_secs(120);

        // สร้าง fixture: วิดีโอ 3s (มี tone audio) + narration wav 2s + music 4s
        async fn mk_ffmpeg(args: Vec<String>, target: &Path) {
            let mut full = vec!["-hide_banner".into(), "-y".into()];
            full.extend(args);
            full.push(target.display().to_string());
            run_ffmpeg(&full, Duration::from_secs(120), || false)
                .await
                .unwrap();
        }
        mk_ffmpeg(
            vec![
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "testsrc=duration=3:size=320x240:rate=10".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "sine=frequency=440:duration=3".into(),
                "-c:v".into(),
                "libx264".into(),
                "-c:a".into(),
                "aac".into(),
            ],
            &video,
        )
        .await;
        mk_ffmpeg(
            vec![
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "sine=frequency=300:duration=2".into(),
            ],
            &wav,
        )
        .await;
        mk_ffmpeg(
            vec![
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "sine=frequency=200:duration=4".into(),
                "-codec:a".into(),
                "libmp3lame".into(),
            ],
            &music,
        )
        .await;

        // narration + music + keep_video_audio → amix ทั้ง 3 แหล่ง
        let args = build_ffmpeg_args(&AssembleInputs {
            video: &video,
            narration: Some(&wav),
            music: Some(&music),
            music_volume: 0.3,
            keep_video_audio: true,
            output: &out,
        });
        run_ffmpeg(&args, short, || false)
            .await
            .expect("ffmpeg assembly should succeed");

        let data = std::fs::read(&out).expect("output exists");
        assert!(data.len() > 1000, "output non-trivial");
        // MP4: ftyp box อยู่ที่ offset 4
        assert_eq!(&data[4..8], b"ftyp", "output is an MP4");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
