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
use crate::media::playback::{Mode, Plan, codec_kind};
use crate::media::{Item, MediaFile, probe};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use worker::{Step, TS_OFFSET_MS, Worker};

/// Length of converted segments; for copied video the minimum, as it is cut at keyframes.
pub const SEGMENT_MS: i64 = 4000;
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
    burn_subtitle: i32,
    video_bitrate: i64,
    max_height: i32,
    hw: String,
    vaapi_device: String,
    preset: String,
}

pub struct Transcoder {
    ffmpeg: String,
    ffprobe: Option<String>,
    root: PathBuf,
    config: Config,
    priority: Vec<String>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    keyframes: Mutex<Vec<(String, Arc<Vec<i64>>)>>,
}

impl Transcoder {
    pub fn new(ffmpeg: String, ffprobe: Option<String>, root: PathBuf, config: Config) -> std::io::Result<Arc<Transcoder>> {
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
            priority: low_priority(),
            sessions: Mutex::default(),
            keyframes: Mutex::default(),
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
            Segments::uniform(f.duration_ms, SEGMENT_MS)
        };
        let transcode = plan.mode == Mode::Transcode;
        let hw = &self.config.hardware_acceleration;
        // VAAPI cannot overlay bitmap subtitles in this simple pipeline; burn-ins use the software path.
        let hw = if !transcode || (plan.burn_subtitle >= 0 && hw == "vaapi") { "none".to_string() } else { hw.clone() };
        let audio_stream = usize::try_from(plan.audio_index).ok().and_then(|i| f.audio_streams.get(i)).map(|a| a.stream_index);
        let job = Job {
            ffmpeg: self.ffmpeg.clone(),
            file: f,
            mode: plan.mode,
            audio_stream,
            burn_subtitle: plan.burn_subtitle,
            video_bitrate: plan.video_bitrate,
            max_height: plan.max_height,
            hw,
            vaapi_device: self.config.vaapi_device.clone(),
            preset: self.config.preset.clone(),
        };
        let id = crate::auth::id();
        let dir = self.root.join(&id);
        std::fs::create_dir_all(&dir).map_err(|e| StartError::Failed(e.to_string()))?;
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

    async fn keyframes_of(&self, f: &MediaFile) -> Arc<Vec<i64>> {
        let Some(ffprobe) = &self.ffprobe else { return Arc::default() };
        let key = format!("{}|{}|{}", f.path, f.size, f.modified);
        if let Some((_, k)) = self.keyframes.lock().unwrap().iter().find(|(k, _)| *k == key) {
            return k.clone();
        }
        let keys = Arc::new(probe::keyframes(ffprobe, Path::new(&f.path), f.video_stream_index, f.duration_ms).await);
        if !keys.is_empty() {
            let mut cache = self.keyframes.lock().unwrap();
            cache.insert(0, (key, keys.clone()));
            cache.truncate(KEYFRAME_CACHE);
        }
        keys
    }
}

/// `nice` and `ionice` in front of ffmpeg, where they exist.
fn low_priority() -> Vec<String> {
    let mut p: Vec<String> = Vec::new();
    if let Some(nice) = ["/usr/bin/nice", "/bin/nice"].iter().find(|x| Path::new(x).is_file()) {
        p.extend([nice.to_string(), "-n".into(), "10".into()]);
    }
    if let Some(ionice) = ["/usr/bin/ionice", "/bin/ionice"].iter().find(|x| Path::new(x).is_file()) {
        p.extend([ionice.to_string(), "-c".into(), "2".into(), "-n".into(), "7".into()]);
    }
    p
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
        if j.hw == "vaapi" {
            let scale = if h > 0 { format!("scale_vaapi=w=-2:h='min({h},ih)'") } else { "null".into() };
            filters.push(format!("{input}format=nv12,hwupload,{scale}"));
        } else {
            let scale = if h > 0 { format!("scale=-2:'min({h},ih)'") } else { "scale=-2:ih".into() };
            filters.push(format!("{input}{scale},format=yuv420p"));
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
        let gop = format!("expr:gte(t,n_forced*{})", seconds(SEGMENT_MS));
        match j.hw.as_str() {
            // Forced keyframes must be IDR frames, or a segment does not start with a picture the
            // browser can decode and the image freezes until the next one.
            "nvenc" => add(&mut c, &["-c:v", "h264_nvenc", "-preset", "p4", "-profile:v", "high", "-forced-idr", "1"]),
            "qsv" => add(&mut c, &["-c:v", "h264_qsv", "-preset", "veryfast", "-profile:v", "high", "-forced_idr", "1"]),
            "vaapi" => add(&mut c, &["-c:v", "h264_vaapi", "-profile:v", "high"]),
            _ => add(&mut c, &["-c:v", "libx264", "-preset", &j.preset, "-profile:v", "high", "-level:v", "4.1", "-sc_threshold", "0"]),
        }
        add(&mut c, &["-b:v", &br, "-maxrate", &br, "-bufsize", &(j.video_bitrate * 2).to_string(), "-force_key_frames", &gop]);
    } else {
        add(&mut c, &["-c:v", "copy"]);
        if codec_kind(&f.video_codec) == "hevc" {
            add(&mut c, &["-tag:v", "hvc1"]);
        }
    }
    if j.audio_stream.is_some() {
        if j.mode == Mode::Remux {
            add(&mut c, &["-c:a", "copy"]);
        } else {
            // Keep the converted sound on the timestamps of the source: fills gaps and absorbs drift
            // (common in TV recordings), which would otherwise move sound and picture apart over time.
            add(&mut c, &["-af", "aresample=async=1:min_hard_comp=0.100000", "-c:a", "aac", "-b:a", "192k", "-ac", "2"]);
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
            burn_subtitle: -1,
            video_bitrate: 0,
            max_height: 0,
            hw: "none".into(),
            vaapi_device: String::new(),
            preset: "veryfast".into(),
        };
        let c = command(&job, 8000).join(" ");
        assert!(c.contains("-noaccurate_seek -ss 8.001 -copyts"));
        assert!(c.contains("-c:v copy -tag:v hvc1"));
        assert!(c.contains("-c:a aac"));
        let t = Job { mode: Mode::Transcode, video_bitrate: 4_000_000, max_height: 720, burn_subtitle: 3, ..job };
        let c = command(&t, 0).join(" ");
        assert!(c.contains("[0:0][0:3]overlay=eof_action=pass,scale=-2:'min(720,ih)',format=yuv420p[v]"));
        assert!(c.contains("-force_key_frames expr:gte(t,n_forced*4.000)"));
        assert!(!c.contains("-ss"));
    }
}
