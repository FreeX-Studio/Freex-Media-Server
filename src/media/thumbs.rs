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

//! Frames from videos as stand-in artwork, made with ffmpeg and cached in `<cache>/thumbs`.

use super::Item;
use super::probe::command;
use crate::auth::now_ms;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;

/// A failed extraction is not retried for this long.
const RETRY_AFTER_MS: i64 = 6 * 3600 * 1000;

pub struct Thumbs {
    dir: PathBuf,
    ffmpeg: Option<String>,
    /// At most two ffmpeg processes at once: a library page can ask for dozens of frames.
    slots: Semaphore,
    /// One extraction per item at a time.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    failed: Mutex<HashMap<String, i64>>,
}

impl Thumbs {
    /// `ffmpeg` is None when it is not available; then only already cached frames are served.
    pub fn new(cache_dir: PathBuf, ffmpeg: Option<String>) -> Thumbs {
        let dir = cache_dir.join("thumbs");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("Cannot create {}: {e}", dir.display());
        }
        Thumbs { dir, ffmpeg, slots: Semaphore::new(2), locks: Mutex::default(), failed: Mutex::default() }
    }

    /// The cached frame of a movie or episode, made if needed; None if there is none.
    pub async fn frame(&self, item: &Item) -> Option<PathBuf> {
        let file = item.file.as_ref()?;
        // The file's modification time is part of the name, so a replaced file gets a new frame.
        let out = self.dir.join(format!("{}-{:x}.jpg", item.id, file.modified));
        if out.is_file() {
            return Some(out);
        }
        let ffmpeg = self.ffmpeg.as_deref()?;
        if self.failed.lock().unwrap().get(&item.id).is_some_and(|t| now_ms() - t < RETRY_AFTER_MS) {
            return None;
        }
        let lock = self.locks.lock().unwrap().entry(item.id.clone()).or_default().clone();
        let result = {
            let _one = lock.lock().await;
            if out.is_file() {
                Some(out)
            } else {
                let _slot = self.slots.acquire().await.ok()?;
                self.extract(ffmpeg, item, &out).await.then_some(out)
            }
        };
        self.locks.lock().unwrap().remove(&item.id);
        if result.is_none() {
            self.failed.lock().unwrap().insert(item.id.clone(), now_ms());
        }
        result
    }

    async fn extract(&self, ffmpeg: &str, item: &Item, out: &std::path::Path) -> bool {
        let Some(file) = &item.file else { return false };
        // Skip intros and black frames: 15 % in, but at most 5 minutes; 30 s if the length is unknown.
        let seek_ms = if file.duration_ms > 0 { (file.duration_ms * 15 / 100).min(300_000) } else { 30_000 };
        let tmp = out.with_extension("tmp.jpg");
        let mut c = command(ffmpeg);
        c.args(["-hide_banner", "-loglevel", "error", "-y", "-ss", &format!("{:.3}", seek_ms as f64 / 1000.0), "-i"])
            .arg(&file.path)
            .args(["-frames:v", "1", "-vf", "scale=640:-2", "-q:v", "4"])
            .arg(&tmp)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let done = matches!(tokio::time::timeout(Duration::from_secs(60), c.status()).await, Ok(Ok(s)) if s.success());
        let ok = done && std::fs::metadata(&tmp).is_ok_and(|m| m.len() > 0) && std::fs::rename(&tmp, out).is_ok();
        let _ = std::fs::remove_file(&tmp);
        ok
    }
}
