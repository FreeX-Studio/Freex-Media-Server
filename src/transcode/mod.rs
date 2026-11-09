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

//! Conversion sessions: HLS with fMP4 segments made by ffmpeg, for files the player cannot play
//! directly. Each session has a [`worker::Worker`]; idle sessions end after a minute.

pub mod worker;

use crate::auth::now_ms;
use crate::config::Transcoder as Config;
use crate::media::playback::{AudioOut, Mode, Plan, codec_kind};
use crate::media::{Item, MediaFile, probe};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use worker::{Step, TS_OFFSET_MS, Worker};

/// Minimum length of segments of copied video, which is cut at keyframes.
pub const SEGMENT_MS: i64 = 4000;
/// Length of converted segments. Short, so that playback starts as soon as the first two seconds
/// are encoded instead of four.
pub const TRANSCODE_SEGMENT_MS: i64 = 2000;
const IDLE_TIMEOUT_MS: i64 = 60_000;
/// Keyframe lists of recently played files kept in memory.
const KEYFRAME_CACHE: usize = 32;

/// Start positions of the segments of a title.
pub struct Segments {
    starts: Vec<i64>,
    duration_ms: i64,
}

impl Segments {
    /// Segments of exactly `length_ms`, for converted video.
    pub fn uniform(duration_ms: i64, length_ms: i64) -> Segments {
        let n = ((duration_ms + length_ms - 1) / length_ms).max(1);
        Segments { starts: (0..n).map(|i| i * length_ms).collect(), duration_ms }
    }

    /// Segments for copied video: each starts at a keyframe and lasts at least `min_ms`.
    pub fn from_keyframes(keyframes: &[i64], duration_ms: i64, min_ms: i64) -> Segments {
        let mut starts = vec![0];
        let mut last = 0;
        for &k in keyframes {
            if k - last >= min_ms && duration_ms - k > 500 {
                starts.push(k);
                last = k;
            }
        }
        Segments { starts, duration_ms }
    }

    pub fn count(&self) -> usize {
        self.starts.len()
    }

    pub fn start(&self, i: usize) -> i64 {
        self.starts[i]
    }

    pub fn end(&self, i: usize) -> i64 {
        self.starts.get(i + 1).copied().unwrap_or(self.duration_ms)
    }

    /// The segment that contains a position.
    pub fn index_at(&self, ms: i64) -> usize {
        self.starts.partition_point(|s| *s <= ms).saturating_sub(1)
    }

    /// HLS playlist (VOD) of all segments; the files are named `s<index>.m4s`.
    pub fn playlist(&self) -> String {
        let longest = (0..self.count()).map(|i| self.end(i) - self.start(i)).max().unwrap_or(0);
        let mut s = format!(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n\
             #EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-MAP:URI=\"init.mp4\"\n",
            (longest + 999) / 1000
        );
        for i in 0..self.count() {
            s.push_str(&format!("#EXTINF:{:.6},\ns{i}.m4s\n", (self.end(i) - self.start(i)) as f64 / 1000.0));
        }
        s.push_str("#EXT-X-ENDLIST\n");
        s
    }
}

pub struct Session {
    pub id: String,
    pub user_id: String,
    pub item_id: String,
    pub mode: Mode,
    dir: PathBuf,
    worker: Mutex<Worker>,
    last_access: AtomicI64,
}

impl Session {
    fn touch(&self) {
        self.last_access.store(now_ms(), Ordering::Relaxed);
    }

    fn worker(&self) -> std::sync::MutexGuard<'_, Worker> {
        self.worker.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn playlist(&self) -> String {
        self.touch();
        self.worker().segments().playlist()
    }

    /// The init segment, once ffmpeg has written it.
    pub async fn init(&self, timeout: Duration) -> Option<PathBuf> {
        self.touch();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match self.worker().init_step() {
                Step::Done(p) => return Some(p),
                Step::Failed => return None,
                Step::Wait => {}
            }
            if tokio::time::Instant::now() > deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// The fragment files of a segment in order, waiting for ffmpeg; None if it cannot be made in time.
    pub async fn segment(&self, i: usize, timeout: Duration) -> Option<Vec<PathBuf>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            self.touch();
            match self.worker().segment_step(i) {
                Step::Done(p) => return Some(p),
                Step::Failed => return None,
                Step::Wait => {}
            }
            if tokio::time::Instant::now() > deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// The last lines of ffmpeg's output.
    pub fn log(&self) -> String {
        self.worker().log()
    }
}

#[derive(Debug)]
pub enum StartError {
    /// All conversion slots are in use.
    Busy,
    Failed(String),
}

/// What the ffmpeg command line is built from.
#[derive(Clone)]
struct Job {
    ffmpeg: String,
    file: MediaFile,
    mode: Mode,
    audio_stream: Option<i32>,
    audio_out: AudioOut,
    burn_subtitle: i32,
    video_bitrate: i64,
    max_height: i32,
    hw: String,
    vaapi_device: String,
    preset: String,
    /// HDR to SDR: None, in software (zscale + tonemap) or with VAAPI (tonemap_vaapi).
    tonemap: ToneMapping,
    /// A text subtitle to burn in, written to `burn.vtt` in the session folder.
    burn_text: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToneMapping {
    None,
    Software,
    Vaapi,
}

use probe::SOFTWARE_TONEMAP;

pub struct Transcoder {
    ffmpeg: String,
    ffprobe: Option<String>,
    root: PathBuf,
    config: Config,
    priority: Vec<String>,
    tonemap: probe::Tonemap,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    keyframes: Mutex<Vec<(String, Arc<Vec<i64>>)>>,
    /// Keyframe lists on disk, so that a film starts fast also after a restart.
    keyframe_dir: Option<PathBuf>,
}

impl Transcoder {
    pub fn new(
        ffmpeg: String,
        ffprobe: Option<String>,
        root: PathBuf,
        config: Config,
        tonemap: probe::Tonemap,
        keyframe_dir: Option<PathBuf>,
    ) -> std::io::Result<Arc<Transcoder>> {
        std::fs::create_dir_all(&root)?;
        clean_directory(&root); // leftovers from a crash
        if config.hardware_acceleration != "none" {
            tracing::info!("Transcoding uses hardware acceleration: {}", config.hardware_acceleration);
        }
        let t = Arc::new(Transcoder {
            ffmpeg,
            ffprobe,
            root,
            config,
            priority: probe::low_priority(),
            tonemap,
            sessions: Mutex::default(),
            keyframes: Mutex::default(),
            keyframe_dir: keyframe_dir.filter(|d| std::fs::create_dir_all(d).is_ok()),
        });
        let reaper = Arc::downgrade(&t);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let Some(t) = reaper.upgrade() else { break };
                t.reap();
            }
        });
        Ok(t)
    }

    pub fn session(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    /// Process ids of the ffmpeg processes of a session.
    pub fn session_pids(&self, id: &str) -> Vec<u32> {
        self.session(id).map(|s| s.worker().pids()).unwrap_or_default()
    }

    /// Process ids of all running conversions.
    pub fn all_pids(&self) -> Vec<u32> {
        let sessions: Vec<Arc<Session>> = self.sessions.lock().unwrap().values().cloned().collect();
        sessions.iter().flat_map(|s| s.worker().pids()).collect()
    }

    pub fn active_sessions(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    /// Starts a conversion for the plan, converting from `start_ms` on first; ends an earlier
    /// session of the same user for the same item.
    pub async fn start(&self, user_id: &str, item: &Item, plan: &Plan, start_ms: i64) -> Result<Arc<Session>, StartError> {
        let old: Vec<String> =
            self.sessions.lock().unwrap().values().filter(|s| s.user_id == user_id && s.item_id == item.id).map(|s| s.id.clone()).collect();
        for id in old {
            self.stop(&id);
        }
        if self.active_sessions() >= self.config.max_sessions as usize {
            return Err(StartError::Busy);
        }
        let f = item.file.clone().ok_or_else(|| StartError::Failed("no file".into()))?;
        let segments = if matches!(plan.mode, Mode::Remux | Mode::Audio) {
            let keys = self.keyframes_of(&f).await;
            if keys.is_empty() {
                return Err(StartError::Failed("the keyframes of the video cannot be read".into()));
            }
            Segments::from_keyframes(&keys, f.duration_ms, SEGMENT_MS)
        } else {
            Segments::uniform(f.duration_ms, TRANSCODE_SEGMENT_MS)
        };
        let transcode = plan.mode == Mode::Transcode;
        let hw = &self.config.hardware_acceleration;
        // VAAPI cannot overlay subtitles in this simple pipeline; burn-ins use the software path.
        let burning = plan.burn_subtitle >= 0 || plan.burn_text.is_some();
        let hw = if !transcode || (burning && hw == "vaapi") { "none".to_string() } else { hw.clone() };
        let audio_stream = usize::try_from(plan.audio_index).ok().and_then(|i| f.audio_streams.get(i)).map(|a| a.stream_index);
        // HDR video converted for an ordinary (SDR) screen, or the colours look washed out.
        let tonemap = if !transcode || !f.tone_mappable() {
            ToneMapping::None
        } else if hw == "vaapi" && self.tonemap.vaapi {
            ToneMapping::Vaapi
        } else if self.tonemap.software {
            ToneMapping::Software
        } else {
            ToneMapping::None
        };
        let job = Job {
            ffmpeg: self.ffmpeg.clone(),
            file: f,
            mode: plan.mode,
            audio_stream,
            audio_out: if plan.mode == Mode::Remux { AudioOut::Copy } else { plan.audio_out },
            burn_subtitle: plan.burn_subtitle,
            video_bitrate: plan.video_bitrate,
            max_height: plan.max_height,
            hw,
            vaapi_device: self.config.vaapi_device.clone(),
            preset: self.config.preset.clone(),
            tonemap,
            burn_text: plan.burn_text.is_some(),
        };
        let id = crate::auth::id();
        let dir = self.root.join(&id);
        std::fs::create_dir_all(&dir).map_err(|e| StartError::Failed(e.to_string()))?;
        if let Some(vtt) = &plan.burn_text {
            std::fs::write(dir.join("burn.vtt"), vtt).map_err(|e| StartError::Failed(e.to_string()))?;
        }
        let count = segments.count();
        let mut worker = Worker::new(dir.clone(), segments, Box::new(move |at| command(&job, at)), self.priority.clone());
        worker.begin(start_ms).map_err(|e| StartError::Failed(format!("cannot start ffmpeg: {e}")))?;
        let s = Arc::new(Session {
            id: id.clone(),
            user_id: user_id.to_string(),
            item_id: item.id.clone(),
            mode: plan.mode,
            dir,
            worker: Mutex::new(worker),
            last_access: AtomicI64::new(now_ms()),
        });
        self.sessions.lock().unwrap().insert(id.clone(), s.clone());
        tracing::info!(
            "Transcode {id} started: '{}' {} from {} s, {count} segments ({})",
            item.title,
            plan.mode.name(),
            start_ms / 1000,
            plan.reason.render("en")
        );
        Ok(s)
    }

    pub fn stop(&self, id: &str) {
        let Some(s) = self.sessions.lock().unwrap().remove(id) else { return };
        s.worker().stop();
        let _ = std::fs::remove_dir_all(&s.dir);
        tracing::info!("Transcode {id} stopped");
    }

    pub fn stop_all(&self) {
        let ids: Vec<String> = self.sessions.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.stop(&id);
        }
    }

    fn reap(&self) {
        let now = now_ms();
        let all: Vec<Arc<Session>> = self.sessions.lock().unwrap().values().cloned().collect();
        for s in all {
            if now - s.last_access.load(Ordering::Relaxed) > IDLE_TIMEOUT_MS {
                self.stop(&s.id);
            } else {
                s.worker().throttle();
            }
        }
    }

    /// Reads the keyframes of a file ahead of time (when its page is opened), so that pressing
    /// play does not wait for it.
    pub fn prefetch(self: &Arc<Self>, f: &MediaFile) {
        if self.ffprobe.is_none() || f.duration_ms <= 0 || f.video_codec.is_empty() {
            return;
        }
        if self.keyframes.lock().unwrap().iter().any(|(k, _)| *k == keyframe_key(f)) {
            return;
        }
        let (t, f) = (self.clone(), f.clone());
        tokio::spawn(async move {
            t.keyframes_of(&f).await;
        });
    }

    async fn keyframes_of(&self, f: &MediaFile) -> Arc<Vec<i64>> {
        let Some(ffprobe) = &self.ffprobe else { return Arc::default() };
        let key = keyframe_key(f);
        if let Some((_, k)) = self.keyframes.lock().unwrap().iter().find(|(k, _)| *k == key) {
            return k.clone();
        }
        let file = self.keyframe_dir.as_ref().map(|d| d.join(format!("{}.txt", short_hash(&key))));
        let stored = match &file {
            Some(p) => tokio::fs::read_to_string(p).await.ok().map(|s| parse_keyframes(&s, &key)).unwrap_or_default(),
            None => Vec::new(),
        };
        let keys = if stored.is_empty() {
            let keys = probe::keyframes(ffprobe, Path::new(&f.path), f.video_stream_index, f.duration_ms).await;
            if let (Some(p), false) = (&file, keys.is_empty()) {
                let tmp = p.with_extension("tmp");
                if tokio::fs::write(&tmp, format_keyframes(&keys, &key)).await.is_ok() {
                    let _ = tokio::fs::rename(&tmp, p).await;
                }
            }
            keys
        } else {
            stored
        };
        let keys = Arc::new(keys);
        if !keys.is_empty() {
            let mut cache = self.keyframes.lock().unwrap();
            cache.retain(|(k, _)| *k != key);
            cache.insert(0, (key, keys.clone()));
            cache.truncate(KEYFRAME_CACHE);
        }
        keys
    }
}

/// Identifies a file's keyframe list: another size or date means another list.
pub fn keyframe_key(f: &MediaFile) -> String {
    format!("{}|{}|{}|{}", f.path, f.size, f.modified, f.video_stream_index)
}

/// FNV-1a of the key, as a file name.
fn short_hash(s: &str) -> String {
    let h = s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{h:016x}")
}

/// The first line holds the key (a hash collision or a changed file is noticed), then one
/// keyframe time in ms per line.
fn format_keyframes(keys: &[i64], key: &str) -> String {
    let mut s = format!("{}\n", key.replace('\n', " "));
    for k in keys {
        s.push_str(&k.to_string());
        s.push('\n');
    }
    s
}

/// The key a stored keyframe list was made for.
pub fn stored_key(s: &str) -> Option<&str> {
    s.lines().next()
}

fn parse_keyframes(s: &str, key: &str) -> Vec<i64> {
    let mut lines = s.lines();
    if lines.next() != Some(&key.replace('\n', " ")) {
        return Vec::new();
    }
    // Written to a temporary file and renamed, so never cut short.
    lines.map_while(|l| l.parse().ok()).collect()
}

/// Removes leftover session folders. Only folders named like a session id are touched, so a
/// misconfigured directory (e.g. the data directory) can never be wiped.
fn clean_directory(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if e.path().is_dir() && name.len() == 24 && name.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

fn seconds(ms: i64) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

/// The ffmpeg command line that converts from `start_ms` (a segment border) on.
fn command(j: &Job, start_ms: i64) -> Vec<String> {
    let f = &j.file;
    let transcode = j.mode == Mode::Transcode;
    let mut c: Vec<String> = [&j.ffmpeg as &str, "-hide_banner", "-loglevel", "warning", "-nostdin", "-y"].map(String::from).to_vec();
    let add = |c: &mut Vec<String>, args: &[&str]| c.extend(args.iter().map(|s| s.to_string()));
    if j.hw == "vaapi" {
        add(&mut c, &["-vaapi_device", &j.vaapi_device]);
    }
    if start_ms > 0 {
        if transcode {
            add(&mut c, &["-ss", &seconds(start_ms)]);
        } else {
            // A copied video starts at the keyframe the segment begins with; audio starts there as
            // well (without -noaccurate_seek ffmpeg drops the sound of the first fragment). 1 ms
            // later, so that rounding cannot make ffmpeg take the keyframe before.
            add(&mut c, &["-noaccurate_seek", "-ss", &seconds(start_ms + 1)]);
        }
    }
    // Keep the timestamps of the source, so that fragments of different runs fit together.
    add(&mut c, &["-copyts", "-start_at_zero", "-i", &f.path]);
    let video_map = if f.video_stream_index >= 0 { format!("0:{}", f.video_stream_index) } else { "0:v:0".into() };
    if transcode {
        let mut filters = Vec::new();
        let mut input = format!("[{video_map}]");
        if j.burn_subtitle >= 0 {
            filters.push(format!("{input}[0:{}]overlay=eof_action=pass", j.burn_subtitle));
            input.clear();
        }
        let h = j.max_height;
        let software_scale = if h > 0 { format!("scale=-2:'min({h},ih)'") } else { "scale=-2:ih".into() };
        let mut software_tonemap = if j.tonemap == ToneMapping::Software { format!("{SOFTWARE_TONEMAP},") } else { String::new() };
        if j.burn_text {
            // ffmpeg runs in a folder of the session (run1, run2, …); the subtitle lies next to them.
            // Rendered after scaling, so the letters have the same size at every quality.
            software_tonemap.push_str("subtitles=../burn.vtt,");
        }
        if j.hw == "vaapi" {
            let scale = if h > 0 { format!("scale_vaapi=w=-2:h='min({h},ih)'") } else { "null".into() };
            match j.tonemap {
                ToneMapping::Vaapi => {
                    filters.push(format!("{input}format=p010,hwupload,tonemap_vaapi=format=nv12:t=bt709:m=bt709:p=bt709,{scale}"))
                }
                // Scaled down first in software, so that tone mapping has fewer pixels to do.
                ToneMapping::Software => filters.push(format!("{input}{software_scale},{software_tonemap}format=nv12,hwupload")),
                ToneMapping::None => filters.push(format!("{input}format=nv12,hwupload,{scale}")),
            }
        } else {
            filters.push(format!("{input}{software_scale},{software_tonemap}format=yuv420p"));
        }
        add(&mut c, &["-filter_complex", &format!("{}[v]", filters.join(",")), "-map", "[v]"]);
    } else {
        add(&mut c, &["-map", &video_map]);
    }
    if let Some(a) = j.audio_stream {
        add(&mut c, &["-map", &format!("0:{a}")]);
    }
    if transcode {
        let br = j.video_bitrate.to_string();
        // Keyframes exactly on the segment grid. t counts from the start of this run (also with
        // -copyts), and every run starts on the grid.
        let gop = format!("expr:gte(t,n_forced*{})", seconds(TRANSCODE_SEGMENT_MS));
        match j.hw.as_str() {
            // Forced keyframes must be IDR frames, or a segment does not start with a picture the
            // browser can decode and the image freezes until the next one.
            "nvenc" => add(&mut c, &["-c:v", "h264_nvenc", "-preset", "p4", "-profile:v", "high", "-forced-idr", "1"]),
            "qsv" => add(&mut c, &["-c:v", "h264_qsv", "-preset", "veryfast", "-profile:v", "high", "-forced_idr", "1"]),
            "vaapi" => add(&mut c, &["-c:v", "h264_vaapi", "-profile:v", "high"]),
            _ => add(&mut c, &["-c:v", "libx264", "-preset", &j.preset, "-profile:v", "high", "-level:v", "4.1", "-sc_threshold", "0"]),
        }
        add(&mut c, &["-b:v", &br, "-maxrate", &br, "-bufsize", &(j.video_bitrate * 2).to_string(), "-force_key_frames", &gop]);
        if j.tonemap != ToneMapping::None {
            add(&mut c, &["-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709"]);
        }
    } else {
        add(&mut c, &["-c:v", "copy"]);
        if codec_kind(&f.video_codec) == "hevc" {
            add(&mut c, &["-tag:v", "hvc1"]);
        }
    }
    if j.audio_stream.is_some() {
        // Keep converted sound on the timestamps of the source: fills gaps and absorbs drift
        // (common in TV recordings), which would otherwise move sound and picture apart over time.
        let resample = ["-af", "aresample=async=1:min_hard_comp=0.100000"];
        match j.audio_out {
            AudioOut::Copy => add(&mut c, &["-c:a", "copy"]),
            AudioOut::AacStereo => {
                add(&mut c, &resample);
                add(&mut c, &["-c:a", "aac", "-b:a", "192k", "-ac", "2"]);
            }
            // 5.1 stays 5.1 (7.1 is folded down to 5.1, as both encoders go up to six channels).
            AudioOut::Ac3 | AudioOut::Eac3 => {
                add(&mut c, &resample);
                let codec = if j.audio_out == AudioOut::Eac3 { "eac3" } else { "ac3" };
                add(&mut c, &["-c:a", codec, "-b:a", "640k", "-ac", "6"]);
            }
        }
    }
    add(
        &mut c,
        &[
            "-sn",
            "-dn",
            "-map_metadata",
            "-1",
            "-map_chapters",
            "-1",
            "-max_muxing_queue_size",
            "2048",
            // Real timestamps in every fragment: no shifting per track, no edit lists (browsers apply
            // them inconsistently), and a fixed offset so that nothing starts below zero.
            "-avoid_negative_ts",
            "disabled",
            "-output_ts_offset",
            &seconds(TS_OFFSET_MS),
            "-f",
            "hls",
            "-hls_time",
            "0.1", // one fragment per keyframe
            "-hls_list_size",
            "0",
            "-hls_playlist_type",
            "event",
            "-hls_segment_type",
            "fmp4",
            "-hls_fmp4_init_filename",
            "init.mp4",
            "-hls_segment_options",
            "use_editlist=0:movflags=+frag_discont+negative_cts_offsets",
            "-hls_segment_filename",
            "seg%05d.m4s",
            "-hls_flags",
            "temp_file+independent_segments",
            "-start_number",
            "0",
            "index.m3u8",
        ],
    );
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyframe_file_roundtrip() {
        let key = "/m/a.mkv|10|20|0";
        let s = format_keyframes(&[0, 2002, 4004], key);
        assert_eq!(parse_keyframes(&s, key), vec![0, 2002, 4004]);
        // Another file (or a changed one) with the same hash is not taken.
        assert!(parse_keyframes(&s, "/m/a.mkv|10|21|0").is_empty());
        assert_eq!(short_hash(key).len(), 16);
    }

    #[test]
    fn segments() {
        let u = Segments::uniform(10_000, 4000);
        assert_eq!((u.count(), u.start(2), u.end(2)), (3, 8000, 10_000));
        assert_eq!(u.index_at(0), 0);
        assert_eq!(u.index_at(7999), 1);
        assert_eq!(u.index_at(50_000), 2);
        let k = Segments::from_keyframes(&[0, 2000, 4100, 6000, 8300, 9800], 10_000, 4000);
        assert_eq!(k.starts, vec![0, 4100, 8300]);
        let p = u.playlist();
        assert!(p.contains("#EXT-X-TARGETDURATION:4\n") && p.contains("#EXTINF:2.000000,\ns2.m4s\n") && p.ends_with("#EXT-X-ENDLIST\n"));
    }

    #[test]
    fn command_lines() {
        let job = Job {
            ffmpeg: "ffmpeg".into(),
            file: MediaFile { path: "/m/a.mkv".into(), video_stream_index: 0, video_codec: "hevc".into(), ..MediaFile::default() },
            mode: Mode::Audio,
            audio_stream: Some(1),
            audio_out: AudioOut::AacStereo,
            burn_subtitle: -1,
            video_bitrate: 0,
            max_height: 0,
            hw: "none".into(),
            vaapi_device: String::new(),
            preset: "veryfast".into(),
            tonemap: ToneMapping::None,
            burn_text: false,
        };
        let c = command(&job, 8000).join(" ");
        assert!(c.contains("-noaccurate_seek -ss 8.001 -copyts"));
        assert!(c.contains("-c:v copy -tag:v hvc1"));
        assert!(c.contains("-c:a aac"));
        let surround = Job { audio_out: AudioOut::Eac3, ..job.clone() };
        assert!(command(&surround, 0).join(" ").contains("-c:a eac3 -b:a 640k -ac 6"));
        let t = Job { mode: Mode::Transcode, video_bitrate: 4_000_000, max_height: 720, burn_subtitle: 3, ..job };
        let c = command(&t, 0).join(" ");
        assert!(c.contains("[0:0][0:3]overlay=eof_action=pass,scale=-2:'min(720,ih)',format=yuv420p[v]"));
        assert!(c.contains("-force_key_frames expr:gte(t,n_forced*2.000)"));
        assert!(!c.contains("-ss"));
        // HDR: tone mapped after scaling, and tagged as BT.709.
        let hdr = Job { burn_subtitle: -1, tonemap: ToneMapping::Software, ..t };
        let c = command(&hdr, 0).join(" ");
        assert!(c.contains(&format!("[0:0]scale=-2:'min(720,ih)',{SOFTWARE_TONEMAP},format=yuv420p[v]")));
        assert!(c.contains("-color_trc bt709"));
        let text = Job { burn_text: true, ..hdr.clone() };
        assert!(command(&text, 0).join(" ").contains(&format!("{SOFTWARE_TONEMAP},subtitles=../burn.vtt,format=yuv420p[v]")));
        let vaapi = Job { hw: "vaapi".into(), tonemap: ToneMapping::Vaapi, ..hdr };
        assert!(
            command(&vaapi, 0).join(" ").contains("format=p010,hwupload,tonemap_vaapi=format=nv12:t=bt709:m=bt709:p=bt709,scale_vaapi")
        );
    }
}
