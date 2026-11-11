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

//! The load of the machine for the server dashboard: CPU of the whole system, of the server and
//! of every ffmpeg process, and the memory of the system and the server. Sampled every few
//! seconds from `/proc` on Linux; on other systems the values stay unknown.
//!
//! The samples are kept for the graphs of the dashboard: every sample of the last hour, and the
//! mean of every minute of the last 24 hours. Nothing is written to disk.

use crate::AppState;
use crate::auth::now_ms;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_secs(3);
/// Samples of the last hour, and minutes of the last day.
const FINE_POINTS: usize = 3600 / 3;
const COARSE_POINTS: usize = 24 * 60;
const MINUTE_MS: i64 = 60_000;

#[derive(Clone, Debug, Default)]
pub struct Load {
    /// Busy share of all cores, 0–100.
    pub cpu: Option<f64>,
    /// CPU of the server process, in percent of one core.
    pub server_cpu: Option<f64>,
    /// Resident memory of the server process in bytes.
    pub memory: Option<u64>,
    /// Memory of the system in use (total minus available) and in total, in bytes.
    pub system_memory: Option<(u64, u64)>,
    /// CPU of processes (ffmpeg) by pid, in percent of one core.
    pub processes: HashMap<u32, f64>,
}

pub struct System {
    pub started: Instant,
    pub cores: usize,
    load: RwLock<Load>,
    history: Mutex<History>,
}

/// One point of the graphs. CPU values are percent of the whole machine (all cores), so the
/// server and ffmpeg are shares of the system line.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub time: i64,
    pub cpu: f32,
    pub server_cpu: f32,
    pub ffmpeg_cpu: f32,
    /// Bytes.
    pub memory: u64,
    pub server_memory: u64,
}

#[derive(Default)]
struct History {
    fine: VecDeque<Point>,
    coarse: VecDeque<Point>,
    /// Samples of the minute that is not over yet.
    minute: Vec<Point>,
    memory_total: u64,
}

impl History {
    fn add(&mut self, p: Point) {
        if self.minute.first().is_some_and(|f| f.time / MINUTE_MS != p.time / MINUTE_MS) {
            let n = self.minute.len() as f32;
            let mean = |f: fn(&Point) -> f32| (self.minute.iter().map(f).sum::<f32>() / n * 10.0).round() / 10.0;
            let mean_bytes = |f: fn(&Point) -> u64| self.minute.iter().map(f).sum::<u64>() / self.minute.len() as u64;
            let m = Point {
                time: self.minute[0].time / MINUTE_MS * MINUTE_MS,
                cpu: mean(|p| p.cpu),
                server_cpu: mean(|p| p.server_cpu),
                ffmpeg_cpu: mean(|p| p.ffmpeg_cpu),
                memory: mean_bytes(|p| p.memory),
                server_memory: mean_bytes(|p| p.server_memory),
            };
            push(&mut self.coarse, m, COARSE_POINTS);
            self.minute.clear();
        }
        self.minute.push(p);
        push(&mut self.fine, p, FINE_POINTS);
    }
}

fn push(q: &mut VecDeque<Point>, p: Point, max: usize) {
    if q.len() == max {
        q.pop_front();
    }
    q.push_back(p);
}

/// The points of a graph and what is needed to draw it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Graph {
    pub supported: bool,
    pub cores: usize,
    pub memory_total: u64,
    /// Milliseconds between points.
    pub interval_ms: i64,
    pub points: Vec<Point>,
}

impl System {
    pub fn new() -> System {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        System { started: Instant::now(), cores, load: RwLock::default(), history: Mutex::default() }
    }

    /// The last hour in every sample, or the last day (`day`) in minutes.
    pub fn graph(&self, day: bool) -> Graph {
        let h = self.history.lock().unwrap();
        let points: Vec<Point> = if day { h.coarse.iter().copied().collect() } else { h.fine.iter().copied().collect() };
        Graph {
            supported: cfg!(target_os = "linux"),
            cores: self.cores,
            memory_total: h.memory_total,
            interval_ms: if day { MINUTE_MS } else { INTERVAL.as_millis() as i64 },
            points,
        }
    }

    fn record(&self, l: &Load) {
        let Some(cpu) = l.cpu else { return };
        let cores = self.cores.max(1) as f64;
        // max(0): the sum of no values is -0.0.
        let share = |v: f64| ((v / cores).max(0.0) * 10.0).round() as f32 / 10.0;
        let (used, total) = l.system_memory.unwrap_or((0, 0));
        let mut h = self.history.lock().unwrap();
        h.memory_total = total;
        h.add(Point {
            time: now_ms(),
            cpu: cpu as f32,
            server_cpu: share(l.server_cpu.unwrap_or(0.0)),
            ffmpeg_cpu: share(l.processes.values().sum()),
            memory: used,
            server_memory: l.memory.unwrap_or(0),
        });
    }

    pub fn load(&self) -> Load {
        self.load.read().unwrap().clone()
    }

    /// Samples in the background for as long as the server runs.
    pub fn spawn(app: Arc<AppState>) {
        if !cfg!(target_os = "linux") {
            return;
        }
        tokio::spawn(async move {
            let mut last: Option<Sample> = None;
            loop {
                let pids = app.transcoder.as_ref().map(|t| t.all_pids()).unwrap_or_default();
                let now = tokio::task::spawn_blocking(move || Sample::take(&pids)).await.ok().flatten();
                if let (Some(a), Some(b)) = (&last, &now) {
                    let load = b.since(a);
                    app.system.record(&load);
                    *app.system.load.write().unwrap() = load;
                }
                last = now;
                tokio::time::sleep(INTERVAL).await;
            }
        });
    }
}

/// Counters at one moment; two of them give the load in between.
struct Sample {
    at: Instant,
    /// Total and idle jiffies of all cores.
    total: u64,
    idle: u64,
    /// utime + stime of the server and of other processes, in clock ticks.
    server: u64,
    processes: HashMap<u32, u64>,
    memory: Option<u64>,
    system_memory: Option<(u64, u64)>,
}

impl Sample {
    fn take(pids: &[u32]) -> Option<Sample> {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let cpu: Vec<u64> = stat.lines().next()?.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
        let total = cpu.iter().sum();
        // idle + iowait
        let idle = cpu.get(3).copied().unwrap_or(0) + cpu.get(4).copied().unwrap_or(0);
        let server = process_ticks("self")?;
        let processes = pids.iter().filter_map(|p| Some((*p, process_ticks(&p.to_string())?))).collect();
        let memory = std::fs::read_to_string("/proc/self/status").ok().and_then(|s| {
            let line = s.lines().find(|l| l.starts_with("VmRSS:"))?;
            line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| kb * 1024)
        });
        let system_memory = std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| meminfo(&s));
        Some(Sample { at: Instant::now(), total, idle, server, processes, memory, system_memory })
    }

    fn since(&self, before: &Sample) -> Load {
        let ticks_per_s = clock_ticks();
        let seconds = self.at.duration_since(before.at).as_secs_f64().max(0.001);
        let total = self.total.saturating_sub(before.total) as f64;
        let idle = self.idle.saturating_sub(before.idle) as f64;
        let per_core = |a: u64, b: u64| (a.saturating_sub(b) as f64 / ticks_per_s / seconds * 100.0 * 10.0).round() / 10.0;
        Load {
            cpu: (total > 0.0).then(|| ((1.0 - idle / total) * 1000.0).round() / 10.0),
            server_cpu: Some(per_core(self.server, before.server)),
            memory: self.memory,
            system_memory: self.system_memory,
            processes: self.processes.iter().filter_map(|(pid, t)| before.processes.get(pid).map(|b| (*pid, per_core(*t, *b)))).collect(),
        }
    }
}

/// Memory in use and in total from `/proc/meminfo`: in use is what is not available, so the page
/// cache, which the system gives back when needed, does not count.
fn meminfo(s: &str) -> Option<(u64, u64)> {
    let kb = |key: &str| -> Option<u64> { s.lines().find(|l| l.starts_with(key))?.split_whitespace().nth(1)?.parse().ok() };
    let total = kb("MemTotal:")?;
    let available = kb("MemAvailable:").or_else(|| Some(kb("MemFree:")? + kb("Cached:").unwrap_or(0)))?;
    Some((total.saturating_sub(available) * 1024, total * 1024))
}

/// utime + stime of a process from `/proc/<pid>/stat`.
fn process_ticks(pid: &str) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is in parentheses and may contain spaces: count fields after it.
    let rest = &s[s.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    // Fields 14 and 15 of the line; here 11 and 12 (state is field 3).
    Some(f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?)
}

fn clock_ticks() -> f64 {
    #[cfg(unix)]
    {
        // SAFETY: sysconf only reads a configuration value.
        let t = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if t > 0 {
            return t as f64;
        }
    }
    100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_from_meminfo() {
        let s = "MemTotal:       16000000 kB\nMemFree:         1000000 kB\nMemAvailable:    6000000 kB\nCached: 4000000 kB\n";
        assert_eq!(meminfo(s), Some((10_000_000 * 1024, 16_000_000 * 1024)));
        assert_eq!(meminfo("MemTotal: 100 kB\nMemFree: 10 kB\nCached: 30 kB\n"), Some((60 * 1024, 100 * 1024)));
    }

    #[test]
    fn history_keeps_minutes() {
        let mut h = History::default();
        let p = |t: i64, cpu: f32| Point { time: t, cpu, memory: 100, ..Point::default() };
        for i in 0..20 {
            h.add(p(i * 3000, 10.0));
        }
        for i in 20..40 {
            h.add(p(i * 3000, 30.0));
        }
        h.add(p(120_000, 50.0));
        assert_eq!(h.fine.len(), 41);
        assert_eq!(h.coarse.len(), 2);
        assert_eq!((h.coarse[0].time, h.coarse[0].cpu), (0, 10.0));
        assert_eq!((h.coarse[1].time, h.coarse[1].cpu), (60_000, 30.0));
        for i in 0..2000 {
            h.add(p(200_000 + i * 3000, 1.0));
        }
        assert_eq!(h.fine.len(), FINE_POINTS);
    }

    #[test]
    fn reads_own_process() {
        if cfg!(target_os = "linux") {
            let a = Sample::take(&[std::process::id()]).expect("proc");
            assert!(a.total > 0 && a.memory.is_some());
            assert!(a.processes.contains_key(&std::process::id()));
            let b = Sample { at: a.at + Duration::from_secs(1), total: a.total + 400, idle: a.idle + 100, server: a.server + 50, ..a };
            let before = Sample::take(&[]).expect("proc");
            let load = b.since(&Sample {
                at: b.at - Duration::from_secs(1),
                total: b.total - 400,
                idle: b.idle - 100,
                server: b.server - 50,
                ..before
            });
            assert_eq!(load.cpu, Some(75.0));
        }
    }
}
