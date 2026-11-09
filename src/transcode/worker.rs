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

//! The background worker of one conversion, like Plex's transcoder: the player sees a complete
//! VOD playlist of fixed segments from the start; the worker runs ffmpeg from a segment border
//! on and hands out segments as soon as their fragments exist.
//!
//! - A jump far ahead or back starts a new ffmpeg run at the requested segment; earlier runs stay
//!   on disk for a while, so jumping back is instant.
//! - ffmpeg writes one fMP4 fragment per keyframe with the source timestamps (`-copyts`), so the
//!   fragments of different runs fit together; the worker reads each fragment's start time from
//!   its `tfdt` box and assembles segments from them.
//! - When ffmpeg is far ahead of the player it is paused (SIGSTOP) and continued later, so a
//!   paused movie does not convert to the end.

use super::Segments;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};

/// All timestamps are shifted by this, so that no track starts below zero (e.g. AAC priming).
pub const TS_OFFSET_MS: i64 = 1000;
/// How far a fragment may start before a segment border and still count as its first one.
const TOLERANCE_MS: i64 = 200;
/// A segment at most this far ahead of the running ffmpeg is waited for instead of restarting.
const WAIT_AHEAD_MS: i64 = 20_000;
/// Pause ffmpeg when it is this far ahead of the player; continue below the lower mark.
const PAUSE_AHEAD_MS: i64 = 120_000;
const RESUME_AHEAD_MS: i64 = 60_000;
/// Folders of earlier runs kept for seeking back.
const KEEP_RUNS: usize = 12;

struct Fragment {
    start_ms: i64,
    path: PathBuf,
}

/// One ffmpeg process, started at a segment border.
struct Run {
    dir: PathBuf,
    child: Child,
    exit: Option<ExitStatus>,
    start_ms: i64,
    fragments: Vec<Fragment>,
    listed: usize,
    timescale: u32,
    paused: bool,
}

impl Run {
    fn poll(&mut self) {
        if self.exit.is_none()
            && let Ok(Some(s)) = self.child.try_wait()
        {
            self.exit = Some(s);
        }
    }

    fn alive(&mut self) -> bool {
        self.poll();
        self.exit.is_none()
    }

    fn finished(&mut self) -> bool {
        self.poll();
        self.exit.is_some_and(|s| s.success())
    }

    fn failed(&mut self) -> bool {
        self.poll();
        self.exit.is_some_and(|s| !s.success())
    }

    fn produced_until(&self) -> i64 {
        self.fragments.last().map_or(self.start_ms, |f| f.start_ms)
    }

    fn kill(&mut self) {
        if self.alive() {
            if self.paused {
                signal(&self.child, Signal::Continue);
            }
            let _ = self.child.kill();
            self.exit = self.child.wait().ok();
        }
    }

    /// Reads the fragments ffmpeg has finished since the last look (it lists them in its playlist).
    fn refresh(&mut self) {
        let Ok(list) = std::fs::read_to_string(self.dir.join("index.m3u8")) else { return };
        let names: Vec<&str> = list.lines().map(str::trim).filter(|l| l.starts_with("seg") && l.ends_with(".m4s")).collect();
        if self.timescale == 0 && names.len() > self.listed {
            self.timescale = video_timescale(&self.dir.join("init.mp4"));
        }
        for name in names.iter().skip(self.listed) {
            let p = self.dir.join(name);
            let Some(start) = fragment_start(&p, self.timescale) else { break }; // not readable yet
            self.fragments.push(Fragment { start_ms: start - TS_OFFSET_MS, path: p });
            self.listed += 1;
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        self.kill();
    }
}

/// What a waiting request should do next.
pub enum Step<T> {
    Done(T),
    Failed,
    Wait,
}

pub struct Worker {
    dir: PathBuf,
    segments: Segments,
    /// ffmpeg command line converting from a position (ms) on, writing into the working directory.
    command: Box<dyn Fn(i64) -> Vec<String> + Send>,
    /// Prefix that lowers the priority of ffmpeg (`nice`, `ionice`), may be empty.
    priority: Vec<String>,
    /// Newest first; the first one is the active run unless `active` is false.
    runs: Vec<Run>,
    active: bool,
    run_number: u32,
    last_requested: usize,
}

impl Worker {
    pub fn new(dir: PathBuf, segments: Segments, command: Box<dyn Fn(i64) -> Vec<String> + Send>, priority: Vec<String>) -> Worker {
        Worker { dir, segments, command, priority, runs: Vec::new(), active: false, run_number: 0, last_requested: 0 }
    }

    pub fn segments(&self) -> &Segments {
        &self.segments
    }

    /// Starts converting at the segment that contains a position.
    pub fn begin(&mut self, position_ms: i64) -> std::io::Result<()> {
        let i = self.segments.index_at(position_ms);
        self.last_requested = i;
        self.start_run(i)
    }

    fn active(&mut self) -> Option<&mut Run> {
        if self.active { self.runs.first_mut() } else { None }
    }

    /// One look for the init segment.
    pub fn init_step(&mut self) -> Step<PathBuf> {
        for r in &mut self.runs {
            r.refresh();
            if !r.fragments.is_empty() {
                return Step::Done(r.dir.join("init.mp4"));
            }
        }
        match self.active().map(|r| r.failed()) {
            Some(false) => Step::Wait,
            _ => Step::Failed,
        }
    }

    /// One look for the fragments of segment `i`, starting or continuing ffmpeg as needed.
    pub fn segment_step(&mut self, i: usize) -> Step<Vec<PathBuf>> {
        if i >= self.segments.count() {
            return Step::Failed;
        }
        self.last_requested = i;
        let (s, e) = (self.segments.start(i), self.segments.end(i));
        for r in &mut self.runs {
            r.refresh();
            if let Some(parts) = collect(r, s, e, i) {
                return Step::Done(parts);
            }
        }
        let seg_start = s;
        let seg_end = e;
        if let Some(r) = self.active() {
            // ffmpeg gave up before writing anything: restarting would only fail again.
            if r.failed() && r.fragments.is_empty() {
                return Step::Failed;
            }
            // Restarted exactly here and still the segment did not come out (no keyframe at its
            // border): give up instead of restarting over and over.
            if r.start_ms == seg_start && passed(r, seg_end) {
                return Step::Failed;
            }
        }
        let covered = self.active().is_some_and(|r| covers(r, seg_start, seg_end));
        if !covered {
            tracing::debug!("Transcode {}: restarting at segment {i}", self.dir.display());
            if self.start_run(i).is_err() {
                return Step::Failed;
            }
        } else if let Some(r) = self.active()
            && r.paused
            && r.produced_until() - seg_start < RESUME_AHEAD_MS
            && signal(&r.child, Signal::Continue)
        {
            r.paused = false;
        }
        Step::Wait
    }

    fn start_run(&mut self, i: usize) -> std::io::Result<()> {
        if let Some(r) = self.active() {
            r.kill();
        }
        self.active = false;
        let start = self.segments.start(i);
        self.run_number += 1;
        let dir = self.dir.join(format!("run{}", self.run_number));
        std::fs::create_dir_all(&dir)?;
        let mut cmd = self.priority.clone();
        cmd.extend((self.command)(start));
        let log = std::fs::File::create(dir.join("ffmpeg.log"))?;
        let mut c = Command::new(&cmd[0]);
        c.args(&cmd[1..]).current_dir(&dir).stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // No console window, and below normal priority (Windows has no nice).
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
            c.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
        }
        let child = c.spawn()?;
        self.runs.insert(0, Run { dir, child, exit: None, start_ms: start, fragments: Vec::new(), listed: 0, timescale: 0, paused: false });
        self.active = true;
        while self.runs.len() > KEEP_RUNS {
            if let Some(old) = self.runs.pop() {
                let dir = old.dir.clone();
                drop(old);
                let _ = std::fs::remove_dir_all(dir);
            }
        }
        Ok(())
    }

    /// Pauses ffmpeg while it is far ahead of the player; called regularly.
    pub fn throttle(&mut self) {
        let requested = self.segments.start(self.last_requested);
        if let Some(r) = self.active() {
            if r.paused || !r.alive() {
                return;
            }
            r.refresh();
            if r.produced_until() - requested > PAUSE_AHEAD_MS && signal(&r.child, Signal::Stop) {
                r.paused = true;
            }
        }
    }

    /// The last lines of ffmpeg's output, for error messages.
    pub fn log(&self) -> String {
        let Some(r) = self.runs.first() else { return String::new() };
        let text = std::fs::read_to_string(r.dir.join("ffmpeg.log")).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(5)..].join("\n")
    }

    pub fn stop(&mut self) {
        self.runs.clear(); // dropping a run ends its ffmpeg
        self.active = false;
    }
}

/// Whether the running ffmpeg will reach the segment soon.
fn covers(r: &mut Run, s: i64, e: i64) -> bool {
    r.alive() && r.start_ms <= s + TOLERANCE_MS && s <= r.produced_until() + WAIT_AHEAD_MS && !passed(r, e)
}

/// Whether a run is already beyond the segment ending at `e` (and so could have delivered it).
fn passed(r: &mut Run, e: i64) -> bool {
    r.produced_until() > e + TOLERANCE_MS || r.finished()
}

/// The fragments of a segment in a run, if the run has all of them.
fn collect(r: &mut Run, s: i64, e: i64, i: usize) -> Option<Vec<PathBuf>> {
    let mut parts = Vec::new();
    let mut complete = false;
    for f in &r.fragments {
        if f.start_ms < s - TOLERANCE_MS {
            continue;
        }
        if f.start_ms >= e - TOLERANCE_MS {
            complete = true;
            break;
        }
        // The first fragment has to start at the border, or the run began inside the segment.
        if parts.is_empty() && f.start_ms > s + TOLERANCE_MS && !(i == 0 && r.start_ms == 0) {
            return None;
        }
        parts.push(f.path.clone());
    }
    if !complete && !parts.is_empty() && r.finished() {
        complete = true; // the end of the title
    }
    (complete && !parts.is_empty()).then_some(parts)
}

enum Signal {
    Stop,
    Continue,
}

/// Pauses or continues a process; false where that is not possible (Windows).
fn signal(child: &Child, s: Signal) -> bool {
    #[cfg(unix)]
    {
        let sig = match s {
            Signal::Stop => libc::SIGSTOP,
            Signal::Continue => libc::SIGCONT,
        };
        let Ok(pid) = libc::pid_t::try_from(child.id()) else { return false };
        // SAFETY: kill(2) with the pid of our own child process and a valid signal number.
        unsafe { libc::kill(pid, sig) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = (child, s);
        false
    }
}

// ---------------------------------------------------------------------- MP4 boxes

fn find(b: &[u8], kind: &[u8; 4], from: usize) -> Option<usize> {
    b.get(from..)?.windows(4).position(|w| w == kind).map(|p| p + from)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Timescale of the video track (the first track) from an init segment, 0 if unknown.
pub fn video_timescale(init: &Path) -> u32 {
    let Ok(b) = std::fs::read(init) else { return 0 };
    (|| {
        let i = find(&b, b"mdhd", 0)?;
        let version = *b.get(i + 4)?;
        u32_at(&b, i + 8 + if version == 1 { 16 } else { 8 })
    })()
    .unwrap_or(0)
}

/// Start of a fragment in ms: the decode time of its video track (track 1).
pub fn fragment_start(fragment: &Path, timescale: u32) -> Option<i64> {
    if timescale == 0 {
        return None;
    }
    let mut b = vec![0u8; 64 * 1024];
    let n = {
        use std::io::Read;
        let mut f = std::fs::File::open(fragment).ok()?;
        let mut total = 0;
        loop {
            let k = f.read(&mut b[total..]).ok()?;
            if k == 0 || total + k == b.len() {
                total += k;
                break;
            }
            total += k;
        }
        total
    };
    let b = &b[..n];
    // moof > traf > (tfhd with the track id, tfdt with the decode time)
    let mut traf = find(b, b"traf", 0);
    while let Some(t) = traf.filter(|t| *t >= 4) {
        let end = n.min(t - 4 + u32_at(b, t - 4)? as usize);
        let (tfhd, tfdt) = (find(b, b"tfhd", t), find(b, b"tfdt", t));
        if let (Some(h), Some(d)) = (tfhd, tfdt)
            && h < end
            && d < end
            && u32_at(b, h + 8) == Some(1)
        {
            let time = if *b.get(d + 4)? == 1 { u64_at(b, d + 8)? } else { u64::from(u32_at(b, d + 8)?) };
            return Some((time as f64 * 1000.0 / f64::from(timescale)).round() as i64);
        }
        traf = find(b, b"traf", end.max(t + 4));
    }
    None
}
