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

//! The server log for the settings page: the last lines are kept in memory next to the normal
//! output (terminal, journald, Windows service log), and the level can be changed while the
//! server runs (`FREEX_LOG` or `info` at start). Nothing is written to disk here.

use serde::Serialize;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Mutex, OnceLock};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::{EnvFilter, Registry, reload};

/// Lines kept.
const KEEP: usize = 5000;
/// Longest line kept; longer ones are cut.
const MAX_LINE: usize = 4000;

/// One line of the log.
#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    /// Counts up from the start of the process; the log view asks for lines after the last it has.
    pub seq: u64,
    pub time: i64,
    pub level: &'static str,
    /// The module that wrote the line, e.g. `freex_media_server::api::jellyfin`.
    pub target: String,
    pub message: String,
}

struct Lines {
    next: u64,
    lines: VecDeque<Entry>,
}

static LINES: Mutex<Lines> = Mutex::new(Lines { next: 1, lines: VecDeque::new() });
static FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();
static LEVEL: Mutex<String> = Mutex::new(String::new());

fn rank(level: &str) -> u8 {
    match level {
        "ERROR" => 1,
        "WARN" => 2,
        "INFO" => 3,
        "DEBUG" => 4,
        _ => 5,
    }
}

/// Lines after `after` of at least the importance of `level` (`error`, `warn`, `info`, `debug`,
/// `trace`) containing `query` (any case); at most `limit`, the newest. Also the newest number.
pub fn lines(after: u64, level: &str, query: &str, limit: usize) -> (Vec<Entry>, u64) {
    let max = rank(&level.to_ascii_uppercase());
    let query = query.to_lowercase();
    let l = LINES.lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<Entry> = l
        .lines
        .iter()
        .rev()
        .filter(|e| e.seq > after && rank(e.level) <= max)
        .filter(|e| query.is_empty() || e.message.to_lowercase().contains(&query) || e.target.to_lowercase().contains(&query))
        .take(limit)
        .cloned()
        .collect();
    out.reverse();
    (out, l.next - 1)
}

/// The level filter the server runs with, as given at the start or set later.
pub fn level() -> String {
    LEVEL.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Sets the level filter while the server runs (EnvFilter syntax: `info`, `debug`,
/// `info,freex_media_server::api::jellyfin=debug`). Until the process ends.
pub fn set_level(spec: &str) -> Result<(), String> {
    let filter = EnvFilter::try_new(spec).map_err(|e| e.to_string())?;
    FILTER.get().ok_or("the log is not set up")?.reload(filter).map_err(|e| e.to_string())?;
    *LEVEL.lock().unwrap_or_else(|e| e.into_inner()) = spec.to_string();
    tracing::info!("Log level set to {spec}");
    Ok(())
}

/// Sets up logging: the filter from `FREEX_LOG` (default `info`), the normal output and the
/// lines in memory.
pub fn init() {
    use tracing_subscriber::prelude::*;
    let spec = std::env::var("FREEX_LOG").ok().filter(|s| EnvFilter::try_new(s).is_ok()).unwrap_or_else(|| "info".into());
    let (filter, handle) = reload::Layer::new(EnvFilter::new(&spec));
    let _ = FILTER.set(handle);
    *LEVEL.lock().unwrap_or_else(|e| e.into_inner()) = spec;
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                // Colours only on a terminal, not in journald or the Windows service log.
                .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout())),
        )
        .with(Memory)
        .init();
}

/// The layer that keeps the lines.
struct Memory;

impl<S: Subscriber> Layer<S> for Memory {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut v = Fields::default();
        event.record(&mut v);
        let mut message = v.message;
        if !v.rest.is_empty() {
            if !message.is_empty() {
                message.push(' ');
            }
            message.push_str(v.rest.trim_start());
        }
        if message.len() > MAX_LINE {
            let mut cut = MAX_LINE;
            while !message.is_char_boundary(cut) {
                cut -= 1;
            }
            message.truncate(cut);
            message.push('…');
        }
        let level = match *event.metadata().level() {
            Level::ERROR => "ERROR",
            Level::WARN => "WARN",
            Level::INFO => "INFO",
            Level::DEBUG => "DEBUG",
            Level::TRACE => "TRACE",
        };
        let mut l = LINES.lock().unwrap_or_else(|e| e.into_inner());
        let seq = l.next;
        l.next += 1;
        if l.lines.len() == KEEP {
            l.lines.pop_front();
        }
        l.lines.push_back(Entry { seq, time: crate::auth::now_ms(), level, target: event.metadata().target().to_string(), message });
    }
}

#[derive(Default)]
struct Fields {
    message: String,
    rest: String,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.rest, " {}={value}", field.name());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.rest, " {}={value:?}", field.name());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_and_filters_lines() {
        // The global subscriber may not be ours in tests; feed the layer directly.
        use tracing_subscriber::prelude::*;
        let sub = tracing_subscriber::registry().with(Memory);
        tracing::subscriber::with_default(sub, || {
            tracing::info!("logbuf-test first");
            tracing::warn!(code = 7, "logbuf-test second");
            tracing::debug!("logbuf-test third");
        });
        let (all, last) = lines(0, "trace", "logbuf-test", 100);
        assert_eq!(all.len(), 3);
        assert_eq!(all[1].message, "logbuf-test second code=7");
        assert_eq!(all[1].level, "WARN");
        assert!(last >= all[2].seq);
        let (warn, _) = lines(0, "warn", "LOGBUF-TEST", 100);
        assert_eq!(warn.len(), 1);
        let (newer, _) = lines(all[0].seq, "info", "logbuf-test", 100);
        assert_eq!(newer.len(), 1, "only lines after the given one");
        let (one, _) = lines(0, "trace", "logbuf-test", 1);
        assert_eq!(one[0].message, "logbuf-test third", "the newest when limited");
    }
}
