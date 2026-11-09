/*
 * Copyright (c) 2026 MirraNET, Niklas Linz. All rights reserved.
 *
 * This file is part of the MirraNET project and is licensed under the
 * GNU Lesser General Public License v3.0 (LGPLv3).
 *
 * You may use, distribute and modify this code under the terms
 * of the LGPLv3 license. You should have received a copy of the
 * license along with this file. If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>
 * or contact: niklas.linz@mirranet.de
 */

//! ffprobe and ffmpeg: finding them, and reading the technical data of a video file.

use super::{AudioStream, Chapter, MediaFile, PROBE_VERSION, SubtitleStream};
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_OUTPUT: usize = 8 * 1024 * 1024;

/// The external programs and whether they work.
pub struct Tools {
    pub ffprobe: String,
    pub ffmpeg: String,
    pub ffprobe_ok: bool,
    pub ffmpeg_ok: bool,
}

impl Tools {
    pub async fn find(ffprobe: &str, ffmpeg: &str) -> Tools {
        let (ffprobe, ffmpeg) = (bundled(ffprobe), bundled(ffmpeg));
        let (ffprobe_ok, ffmpeg_ok) = tokio::join!(works(&ffprobe), works(&ffmpeg));
        if !ffprobe_ok {
            tracing::warn!("ffprobe not found ('{ffprobe}'): files are indexed without duration and codec info");
        }
        if !ffmpeg_ok {
            tracing::warn!("ffmpeg not found ('{ffmpeg}'): no generated thumbnails and no conversion");
        }
        Tools { ffprobe, ffmpeg, ffprobe_ok, ffmpeg_ok }
    }
}

/// A bare program name is taken from the `ffmpeg` folder next to the server if there is one
/// (the Windows installer puts ffmpeg there), otherwise from the PATH as given.
fn bundled(program: &str) -> String {
    if program.contains(['/', '\\']) {
        return program.to_string();
    }
    let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) else { return program.to_string() };
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let candidate = dir.join("ffmpeg").join(format!("{program}{ext}"));
    if candidate.is_file() { candidate.to_string_lossy().into_owned() } else { program.to_string() }
}

/// A command for an external program: no console window on Windows, killed when dropped.
pub fn command(program: &str) -> Command {
    let mut c = Command::new(program);
    c.stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// `nice` and `ionice` in front of a command, where they exist (Linux and other Unix systems).
pub fn low_priority() -> Vec<String> {
    let mut p: Vec<String> = Vec::new();
    if let Some(nice) = ["/usr/bin/nice", "/bin/nice"].iter().find(|x| Path::new(x).is_file()) {
        p.extend([nice.to_string(), "-n".into(), "10".into()]);
    }
    if let Some(ionice) = ["/usr/bin/ionice", "/bin/ionice"].iter().find(|x| Path::new(x).is_file()) {
        p.extend([ionice.to_string(), "-c".into(), "2".into(), "-n".into(), "7".into()]);
    }
    p
}

/// Like [`command`], for background work that must not slow down playback: below normal
/// priority for CPU and disk.
pub fn background(program: &str) -> Command {
    let prefix = low_priority();
    let mut c = match prefix.split_first() {
        Some((first, rest)) => {
            let mut c = Command::new(first);
            c.args(rest).arg(program);
            c
        }
        None => Command::new(program),
    };
    c.stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        c.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }
    c
}

async fn works(program: &str) -> bool {
    let mut c = command(program);
    c.arg("-version").stdout(Stdio::null()).stderr(Stdio::null());
    matches!(tokio::time::timeout(Duration::from_secs(10), c.status()).await, Ok(Ok(s)) if s.success())
}

/// ffprobe's JSON for a file, or None if it cannot be read.
pub async fn probe(ffprobe: &str, video: &Path) -> Option<Value> {
    let mut c = command(ffprobe);
    c.args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams", "-show_chapters"])
        .arg(video)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let out = match tokio::time::timeout(PROBE_TIMEOUT, c.output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            tracing::debug!("ffprobe failed on {}: {e}", video.display());
            return None;
        }
        Err(_) => {
            tracing::warn!("ffprobe timed out on {}", video.display());
            return None;
        }
    };
    if !out.status.success() || out.stdout.is_empty() || out.stdout.len() > MAX_OUTPUT {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// Times of the video keyframes of a file in ms from its start (as ffmpeg's `-ss` counts),
/// sorted; empty if they cannot be read. Conversions that copy the video can only cut there.
///
/// Instead of reading the whole file, ffprobe seeks to every other second and reports the
/// keyframe it lands on; that uses the file's index and is fast even on a network share. Closer
/// keyframes may be missed, which is fine: segments only need some keyframes as borders. Files
/// without an index are read completely.
pub async fn keyframes(ffprobe: &str, video: &Path, stream_index: i32, duration_ms: i64) -> Vec<i64> {
    if duration_ms <= 0 {
        return Vec::new();
    }
    let start = start_time(ffprobe, video).await;
    let intervals: Vec<String> = (0..duration_ms).step_by(2000).map(|t| format!("{:.3}%+#1", start + t as f64 / 1000.0)).collect();
    let keys = keyframe_packets(ffprobe, video, stream_index, Some(&intervals.join(",")), start).await;
    // Without an index every seek lands at the start: then the file has to be read once.
    if (keys.len() as i64) < (duration_ms / 120_000).max(2) {
        return keyframe_packets(ffprobe, video, stream_index, None, start).await;
    }
    keys
}

async fn keyframe_packets(ffprobe: &str, video: &Path, stream_index: i32, intervals: Option<&str>, start: f64) -> Vec<i64> {
    let stream = if stream_index >= 0 { stream_index.to_string() } else { "v:0".to_string() };
    let mut c = command(ffprobe);
    c.args(["-v", "error", "-select_streams", &stream, "-show_entries", "packet=pts_time,flags", "-of", "csv=p=0"]);
    if let Some(i) = intervals {
        c.args(["-read_intervals", i]);
    }
    c.arg(video).stdout(Stdio::piped()).stderr(Stdio::null());
    let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(120), c.output()).await else { return Vec::new() };
    let mut keys: Vec<i64> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (t, flags) = l.split_once(',')?;
            if !flags.starts_with('K') || t == "N/A" {
                return None;
            }
            Some((((t.trim().parse::<f64>().ok()? - start) * 1000.0).round() as i64).max(0))
        })
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// Start of the file's timestamps in seconds (often 0, not for MPEG-TS recordings).
async fn start_time(ffprobe: &str, video: &Path) -> f64 {
    let mut c = command(ffprobe);
    c.args(["-v", "error", "-show_entries", "format=start_time", "-of", "csv=p=0"]).arg(video).stdout(Stdio::piped()).stderr(Stdio::null());
    match tokio::time::timeout(Duration::from_secs(10), c.output()).await {
        Ok(Ok(o)) => String::from_utf8_lossy(&o.stdout).trim().parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn s(v: &Value, key: &str) -> String {
    match &v[key] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Numbers come as JSON numbers or as strings ("bit_rate": "5000000").
fn n(v: &Value, key: &str) -> i64 {
    match &v[key] {
        Value::Number(n) => n.as_i64().unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        Value::String(s) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn seconds_ms(v: &Value, key: &str) -> i64 {
    (s(v, key).trim().parse::<f64>().unwrap_or(0.0) * 1000.0).round() as i64
}

/// ffprobe reports ISO 639-2 codes ("ger", "deu", "eng"); "und" means unknown.
fn language(tags: &Value) -> String {
    let l = s(tags, "language").trim().to_lowercase();
    if l == "und" { String::new() } else { l }
}

/// Fills the technical fields of a file from ffprobe's JSON.
pub fn apply(json: &Value, f: &mut MediaFile) {
    let format = &json["format"];
    f.duration_ms = seconds_ms(format, "duration");
    f.bitrate = n(format, "bit_rate");
    f.format_name = s(format, "format_name");
    f.audio_streams.clear();
    f.subtitle_streams.clear();
    let empty = Vec::new();
    let streams = json["streams"].as_array().unwrap_or(&empty);
    let mut have_video = false;
    for st in streams {
        let disposition = &st["disposition"];
        let tags = &st["tags"];
        let index = n(st, "index") as i32;
        let title = s(tags, "title");
        let is_default = n(disposition, "default") == 1;
        match s(st, "codec_type").as_str() {
            "video" if !have_video && n(disposition, "attached_pic") != 1 => {
                f.video_codec = s(st, "codec_name");
                f.video_profile = s(st, "profile");
                f.pixel_format = s(st, "pix_fmt");
                f.width = n(st, "width") as i32;
                f.height = n(st, "height") as i32;
                f.video_stream_index = index;
                have_video = true;
            }
            "audio" => f.audio_streams.push(AudioStream {
                stream_index: index,
                codec: s(st, "codec_name"),
                channels: n(st, "channels") as i32,
                language: language(tags),
                title,
                is_default,
                bitrate: n(st, "bit_rate"),
            }),
            "subtitle" => f.subtitle_streams.push(SubtitleStream {
                stream_index: index,
                codec: s(st, "codec_name"),
                language: language(tags),
                forced: n(disposition, "forced") == 1 || title.to_lowercase().contains("forced"),
                title,
                is_default,
            }),
            _ => {}
        }
    }
    // The browser plays the first audio track in direct play, so that is what is reported.
    if let Some(a) = f.audio_streams.first() {
        f.audio_codec = a.codec.clone();
        f.audio_channels = a.channels;
    }
    if f.duration_ms == 0 {
        // Some containers only carry the duration on the stream.
        f.duration_ms = streams.iter().map(|st| seconds_ms(st, "duration")).find(|d| *d > 0).unwrap_or(0);
    }
    f.chapters = chapters(json["chapters"].as_array().unwrap_or(&empty), f.duration_ms);
    f.probe_version = PROBE_VERSION;
    f.probed = true;
}

/// The chapters of a file in order. Chapters past the end are dropped, gaps and overlaps are
/// closed (a chapter runs until the next one starts), and a single chapter over the whole file
/// says nothing, so it is dropped as well.
fn chapters(list: &[Value], duration_ms: i64) -> Vec<Chapter> {
    let mut out: Vec<Chapter> = list
        .iter()
        .map(|c| Chapter {
            start_ms: seconds_ms(c, "start_time").max(0),
            end_ms: seconds_ms(c, "end_time"),
            title: s(&c["tags"], "title").trim().to_string(),
        })
        .filter(|c| duration_ms <= 0 || c.start_ms < duration_ms - 1000)
        .collect();
    out.sort_by_key(|c| c.start_ms);
    out.dedup_by_key(|c| c.start_ms);
    let starts: Vec<i64> = out.iter().skip(1).map(|c| c.start_ms).collect();
    for (i, c) in out.iter_mut().enumerate() {
        let end = starts.get(i).copied().unwrap_or(if duration_ms > 0 { duration_ms } else { c.end_ms });
        c.end_ms = end.max(c.start_ms);
    }
    if out.len() == 1 {
        out.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_chapters() {
        let list = vec![
            json!({"start_time": "600.5", "end_time": "1300", "tags": {"title": "Zwei "}}),
            json!({"start_time": "0.000000", "end_time": "590", "tags": {"title": "Eins"}}),
            json!({"start_time": "1300", "end_time": "1800"}),
            json!({"start_time": "1799.5", "end_time": "1800", "tags": {"title": "Past the end"}}),
        ];
        let c = chapters(&list, 1_800_000);
        assert_eq!(c.len(), 3);
        assert_eq!((c[0].start_ms, c[0].end_ms, c[0].title.as_str()), (0, 600_500, "Eins"));
        assert_eq!((c[1].start_ms, c[1].end_ms, c[1].title.as_str()), (600_500, 1_300_000, "Zwei"));
        assert_eq!((c[2].end_ms, c[2].title.as_str()), (1_800_000, ""));
        // One chapter for the whole file is no help.
        assert!(chapters(&list[1..2], 1_800_000).is_empty());
    }

    #[test]
    fn applies_streams() {
        let j = json!({
            "format": {"duration": "5400.250", "bit_rate": "8000000", "format_name": "matroska,webm"},
            "streams": [
                {"index": 0, "codec_type": "video", "codec_name": "mjpeg", "disposition": {"attached_pic": 1}},
                {"index": 1, "codec_type": "video", "codec_name": "h264", "profile": "High", "pix_fmt": "yuv420p",
                 "width": 1920, "height": 800},
                {"index": 2, "codec_type": "audio", "codec_name": "eac3", "channels": 6, "tags": {"language": "ger"},
                 "disposition": {"default": 1}},
                {"index": 3, "codec_type": "subtitle", "codec_name": "hdmv_pgs_subtitle", "tags": {"language": "und", "title": "Forced"}}
            ]
        });
        let mut f = MediaFile::default();
        apply(&j, &mut f);
        assert_eq!(f.duration_ms, 5_400_250);
        assert_eq!(f.bitrate, 8_000_000);
        assert_eq!((f.video_codec.as_str(), f.video_stream_index, f.width), ("h264", 1, 1920));
        assert_eq!((f.audio_codec.as_str(), f.audio_channels), ("eac3", 6));
        assert!(f.audio_streams[0].is_default);
        assert_eq!(f.audio_streams[0].language, "ger");
        assert!(f.subtitle_streams[0].forced && f.subtitle_streams[0].language.is_empty());
        assert!(!f.subtitle_streams[0].text_based());
        assert!(f.probe_current());
    }
}
