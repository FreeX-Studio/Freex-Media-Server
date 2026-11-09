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
//! of every ffmpeg process, and the server's memory. Sampled every few seconds from `/proc` on
//! Linux; on other systems the values stay unknown.

use crate::AppState;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Default)]
pub struct Load {
    /// Busy share of all cores, 0–100.
    pub cpu: Option<f64>,
    /// CPU of the server process, in percent of one core.
    pub server_cpu: Option<f64>,
    /// Resident memory of the server process in bytes.
    pub memory: Option<u64>,
    /// CPU of processes (ffmpeg) by pid, in percent of one core.
    pub processes: HashMap<u32, f64>,
}

pub struct System {
    pub started: Instant,
    pub cores: usize,
    load: RwLock<Load>,
}

impl System {
    pub fn new() -> System {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        System { started: Instant::now(), cores, load: RwLock::default() }
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
                    *app.system.load.write().unwrap() = b.since(a);
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
        Some(Sample { at: Instant::now(), total, idle, server, processes, memory })
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
            processes: self.processes.iter().filter_map(|(pid, t)| before.processes.get(pid).map(|b| (*pid, per_core(*t, *b)))).collect(),
        }
    }
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
