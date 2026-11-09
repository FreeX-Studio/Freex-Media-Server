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

//! The nightly maintenance window: a backup of the database, clean-up of the caches and a rescan
//! of all libraries, which also refreshes metadata and runs the intro detection and the seek
//! previews for new files. Starts at `[maintenance] start` in the server's time zone, or on
//! request from the dashboard.

use crate::AppState;
use crate::auth::now_ms;
use crate::media::scanner::Scanner;
use chrono::{Local, NaiveTime, TimeDelta};
use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the last run did.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub started_at: i64,
    pub finished_at: i64,
    /// File name of the backup, empty if none was made.
    pub backup: String,
    pub backup_error: String,
    /// Cache files removed and bytes freed.
    pub removed_files: usize,
    pub freed_bytes: u64,
    pub libraries_scanned: usize,
}

#[derive(Default)]
pub struct Maintenance {
    running: AtomicBool,
    last: Mutex<Option<Report>>,
}

impl Maintenance {
    pub fn running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn last(&self) -> Option<Report> {
        self.last.lock().unwrap().clone()
    }

    /// Waits for the start time every day and runs the maintenance.
    pub fn spawn(app: Arc<AppState>) {
        let m = &app.config.maintenance;
        if !m.enabled {
            return;
        }
        let Some((h, min)) = m.start_time() else {
            tracing::warn!("maintenance.start '{}' is not a time of day (HH:MM); no nightly maintenance", m.start);
            return;
        };
        let at = NaiveTime::from_hms_opt(h, min, 0).expect("checked");
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(until(at)).await;
                Maintenance::run(&app).await;
                // Not twice in the same minute.
                tokio::time::sleep(Duration::from_secs(61)).await;
            }
        });
    }

    /// One run; false if one is already running.
    pub async fn run(app: &Arc<AppState>) -> bool {
        if app.maintenance.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        tracing::info!("Maintenance started");
        let mut r = Report { started_at: now_ms(), ..Report::default() };
        let keep = app.config.maintenance.backups as usize;
        if keep > 0 {
            let dir = app.config.data_dir().join("backups");
            let db = app.clone();
            let result = tokio::task::spawn_blocking(move || backup(&db, &dir, keep)).await.unwrap_or_else(|e| Err(e.to_string()));
            match result {
                Ok(name) => r.backup = name,
                Err(e) => {
                    tracing::warn!("Database backup failed: {e}");
                    r.backup_error = e;
                }
            }
        }
        let a = app.clone();
        if let Ok((files, bytes)) = tokio::task::spawn_blocking(move || clean_caches(&a)).await {
            (r.removed_files, r.freed_bytes) = (files, bytes);
        }
        let libraries = app.db.libraries();
        r.libraries_scanned = libraries.len();
        Scanner::scan_all(app);
        r.finished_at = now_ms();
        tracing::info!(
            "Maintenance done: backup {}, {} cache files removed ({} MB), {} libraries rescanned",
            if r.backup.is_empty() { "skipped" } else { &r.backup },
            r.removed_files,
            r.freed_bytes / 1_048_576,
            r.libraries_scanned
        );
        *app.maintenance.last.lock().unwrap() = Some(r);
        app.maintenance.running.store(false, Ordering::SeqCst);
        true
    }
}

/// Time until the next `at` local time.
fn until(at: NaiveTime) -> Duration {
    let now = Local::now();
    let today = now.date_naive().and_time(at);
    let next = if today > now.naive_local() { today } else { today + TimeDelta::days(1) };
    (next - now.naive_local()).to_std().unwrap_or(Duration::from_secs(60))
}

/// `freex-YYYY-MM-DD-HHMM.db` in `dir`; keeps the newest `keep` backups.
fn backup(app: &AppState, dir: &Path, keep: usize) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = format!("freex-{}.db", Local::now().format("%Y-%m-%d-%H%M"));
    let path = dir.join(&name);
    if path.exists() {
        return Ok(name);
    }
    app.db.backup_to(&path)?;
    let mut old: Vec<_> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("freex-") && n.ends_with(".db"))
                .collect()
        })
        .unwrap_or_default();
    // The names sort by time.
    old.sort();
    let surplus = old.len().saturating_sub(keep);
    for n in &old[..surplus] {
        let _ = std::fs::remove_file(dir.join(n));
    }
    Ok(name)
}

/// Removes generated frames and extracted subtitles of entries that no longer exist or whose file
/// has changed (their names carry the item id and the file's modification time).
fn clean_caches(app: &AppState) -> (usize, u64) {
    let idx = app.catalog.snapshot();
    let current = |name: &str| {
        // "<id>-<mtime hex>.jpg" or "<id>-<stream>-<mtime hex>.vtt"
        let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
        let (Some(id), Some(last)) = (stem.split('-').next(), stem.rsplit('-').next()) else { return true };
        match idx.item(id).and_then(|it| it.file.as_ref()) {
            Some(f) => format!("{:x}", f.modified) == last,
            None => false,
        }
    };
    let mut removed = (0, 0);
    let cache = app.config.cache_dir();
    for dir in ["thumbs", "subtitles"] {
        let Ok(entries) = std::fs::read_dir(cache.join(dir)) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(meta) = e.metadata() else { continue };
            // Leave files that are being written alone.
            if !meta.is_file() || name.contains(".tmp") || current(&name) {
                continue;
            }
            if std::fs::remove_file(e.path()).is_ok() {
                removed.0 += 1;
                removed.1 += meta.len();
            }
        }
    }
    // Keyframe lists of files that are gone or changed.
    let keys: std::collections::HashSet<String> = idx
        .all()
        .flat_map(|it| it.file.iter().chain(it.versions.iter().map(|v| &v.file)))
        .map(|f| crate::transcode::keyframe_key(f).replace('\n', " "))
        .collect();
    if let Ok(entries) = std::fs::read_dir(cache.join("keyframes")) {
        for e in entries.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if e.file_name().to_string_lossy().ends_with(".tmp") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
            if !meta.is_file() || crate::transcode::stored_key(&text).is_some_and(|k| keys.contains(k)) {
                continue;
            }
            if std::fs::remove_file(e.path()).is_ok() {
                removed.0 += 1;
                removed.1 += meta.len();
            }
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_start() {
        let soon = (Local::now() + TimeDelta::minutes(5)).time();
        let d = until(soon);
        assert!(d <= Duration::from_secs(301) && d >= Duration::from_secs(240), "{d:?}");
        let past = (Local::now() - TimeDelta::minutes(5)).time();
        assert!(until(past) > Duration::from_secs(23 * 3600));
    }
}
