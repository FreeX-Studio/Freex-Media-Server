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

//! Seek preview thumbnails: small frames at a fixed interval, shown above the seek bar while
//! scrubbing. ffmpeg decodes only the keyframes of a file and tiles the frames into one JPEG
//! sprite sheet per title, stored in `<cache>/previews` with a small JSON file describing the
//! grid. Runs in the background after scans, one file at a time and at low priority.

use super::Item;
use super::probe::background;
use crate::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Width of one frame in pixels.
pub const FRAME_WIDTH: i32 = 240;
/// Shortest interval between two frames.
const MIN_INTERVAL_S: i64 = 10;
/// Longer titles get a longer interval, so a sheet holds at most about this many frames.
const MAX_FRAMES: i64 = 600;
const COLUMNS: i64 = 10;
/// Titles shorter than this get no previews.
const MIN_DURATION_MS: i64 = 60_000;
/// Upper limit for one file; a 4K film decoded on a slow machine can take a while.
const TIMEOUT: Duration = Duration::from_secs(45 * 60);

/// The grid of a sprite sheet: frame `n` shows the position `n * interval_ms` and sits in
/// column `n % columns`, row `n / columns`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub interval_ms: i64,
    pub width: u32,
    pub height: u32,
    pub columns: u32,
    pub rows: u32,
    pub count: u32,
}

/// The number of frames really in the sheet: ffmpeg sometimes stops a frame or two before the
/// end, and the tile filter pads the rest of the grid with black. Trailing black tiles are not
/// counted (a black last frame of a film is no loss either).
fn filled(sheet: &image::RgbImage, width: u32, height: u32, count: u32) -> u32 {
    let black = |n: u32| {
        let (x0, y0) = ((n % COLUMNS as u32) * width, (n / COLUMNS as u32) * height);
        (0..8).all(|i| {
            (0..8).all(|j| {
                let p = sheet.get_pixel(x0 + (2 * i + 1) * width / 16, y0 + (2 * j + 1) * height / 16);
                p.0.iter().all(|c| *c < 24)
            })
        })
    };
    let mut n = count;
    while n > 1 && black(n - 1) {
        n -= 1;
    }
    n
}

/// Interval, number of frames and rows for a title of `duration_ms`.
pub fn grid(duration_ms: i64) -> (i64, i64, i64) {
    let seconds = (duration_ms + 999) / 1000;
    let interval = MIN_INTERVAL_S.max((seconds + MAX_FRAMES - 1) / MAX_FRAMES);
    // The fps filter emits frames at 0, I, 2I, ... before the end.
    let count = (duration_ms - 1) / (interval * 1000) + 1;
    let rows = (count + COLUMNS - 1) / COLUMNS;
    (interval, count, rows)
}

pub struct Previews {
    dir: PathBuf,
    ffmpeg: String,
    queued: Mutex<HashSet<String>>,
    serial: tokio::sync::Mutex<()>,
}

impl Previews {
    pub fn new(cache_dir: &Path, ffmpeg: String) -> Previews {
        let dir = cache_dir.join("previews");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("Cannot create {}: {e}", dir.display());
        }
        Previews { dir, ffmpeg, queued: Mutex::default(), serial: tokio::sync::Mutex::new(()) }
    }

    /// Movies and episodes with a picture and a length worth scrubbing through.
    pub fn wanted(item: &Item) -> bool {
        matches!(item.kind, super::Kind::Movie | super::Kind::Episode)
            && item.file.as_ref().is_some_and(|f| f.duration_ms >= MIN_DURATION_MS && f.width > 0 && !f.video_codec.is_empty())
    }

    /// File name stem for the item's current file; a replaced file gets new previews.
    fn stem(item: &Item) -> Option<String> {
        Some(format!("{}-{:x}", item.id, item.file.as_ref()?.modified))
    }

    /// The sprite sheet and its grid, if they exist for the item's current file.
    pub fn get(&self, item: &Item) -> Option<(PathBuf, Layout)> {
        let stem = Self::stem(item)?;
        let image = self.dir.join(format!("{stem}.jpg"));
        let layout = std::fs::read(self.dir.join(format!("{stem}.json"))).ok()?;
        let layout: Layout = serde_json::from_slice(&layout).ok()?;
        image.is_file().then_some((image, layout))
    }

    pub fn layout(&self, item: &Item) -> Option<Layout> {
        self.get(item).map(|(_, l)| l)
    }

    pub fn busy(&self) -> bool {
        !self.queued.lock().unwrap().is_empty()
    }

    /// Queues the titles of a library that have no previews yet; with `force` all of them.
    pub fn generate_library(app: &Arc<AppState>, library_id: &str, force: bool) {
        let Some(service) = &app.previews else { return };
        if !service.queued.lock().unwrap().insert(library_id.to_string()) {
            return;
        }
        let (app, id) = (app.clone(), library_id.to_string());
        tokio::spawn(async move {
            let service = app.previews.as_ref().expect("checked");
            {
                let _turn = service.serial.lock().await;
                let idx = app.catalog.snapshot();
                let mut made = 0;
                // Movies with several versions: the main entry has the best version's file, the
                // other versions get their own sheets.
                let mut todo: Vec<&Arc<Item>> = Vec::new();
                for it in idx.of_library(&id).filter(|it| Self::wanted(it)) {
                    todo.push(it);
                    todo.extend(idx.versions_of(it).into_iter().skip(1));
                }
                for it in todo {
                    if !force && (service.get(it).is_some() || service.failed(it)) {
                        continue;
                    }
                    // The item may have been changed or removed in the meantime.
                    let Some(current) = app.catalog.snapshot().item(&it.id).cloned() else { continue };
                    if service.make(&current).await {
                        made += 1;
                    }
                }
                if made > 0 {
                    tracing::info!("Seek previews: {made} new sheets");
                }
                service.clean(&app);
            }
            service.queued.lock().unwrap().remove(&id);
        });
    }

    fn failed(&self, item: &Item) -> bool {
        Self::stem(item).is_some_and(|s| self.dir.join(format!("{s}.failed")).is_file())
    }

    /// Makes the sprite sheet of one title. A failure is remembered until the file changes.
    async fn make(&self, item: &Item) -> bool {
        let (Some(file), Some(stem)) = (&item.file, Self::stem(item)) else { return false };
        let (interval, count, rows) = grid(file.duration_ms);
        let image = self.dir.join(format!("{stem}.jpg"));
        let tmp = self.dir.join(format!("{stem}.tmp.jpg"));
        let mut c = background(&self.ffmpeg);
        // Only keyframes are decoded: much faster than every frame, and close enough for a preview.
        c.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-skip_frame", "nokey", "-i"])
            .arg(&file.path)
            .args(["-map", "0:v:0", "-an", "-sn", "-dn", "-vf"])
            .arg(format!("fps=1/{interval}:round=down,scale={FRAME_WIDTH}:-2:flags=fast_bilinear,tile={COLUMNS}x{rows}"))
            .args(["-frames:v", "1", "-q:v", "5", "-update", "1"])
            .arg(&tmp)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let started = std::time::Instant::now();
        let done = matches!(tokio::time::timeout(TIMEOUT, c.status()).await, Ok(Ok(s)) if s.success());
        let sheet = if done {
            let path = tmp.clone();
            tokio::task::spawn_blocking(move || image::open(path).ok().map(|i| i.to_rgb8())).await.ok().flatten()
        } else {
            None
        };
        let ok = match sheet {
            Some(sheet) if sheet.width() >= COLUMNS as u32 && sheet.height() >= rows as u32 => {
                let (width, height) = (sheet.width() / COLUMNS as u32, sheet.height() / rows as u32);
                let layout = Layout {
                    interval_ms: interval * 1000,
                    width,
                    height,
                    columns: COLUMNS as u32,
                    rows: rows as u32,
                    count: filled(&sheet, width, height, count as u32),
                };
                let json = serde_json::to_vec(&layout).expect("serializable");
                std::fs::rename(&tmp, &image).is_ok() && std::fs::write(self.dir.join(format!("{stem}.json")), json).is_ok()
            }
            _ => false,
        };
        let _ = std::fs::remove_file(&tmp);
        if ok {
            tracing::debug!("Seek previews for '{}': {count} frames in {} s", item.title, started.elapsed().as_secs());
            self.remove_others(&item.id, &stem);
        } else {
            tracing::debug!("No seek previews for {}", file.path);
            let _ = std::fs::write(self.dir.join(format!("{stem}.failed")), b"");
        }
        ok
    }

    /// Removes the files of older versions of an item.
    fn remove_others(&self, id: &str, keep: &str) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let prefix = format!("{id}-");
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) && !name.starts_with(keep) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    /// Removes previews of items that are no longer in any library.
    fn clean(&self, app: &AppState) {
        let idx = app.catalog.snapshot();
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some((id, _)) = name.rsplit_once('-') else { continue };
            if idx.item(id).is_none() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_sizes() {
        // 45 minutes: every 10 s, 270 frames in 27 rows.
        assert_eq!(grid(45 * 60_000), (10, 270, 27));
        // Just over a whole interval gets one more frame.
        assert_eq!(grid(45 * 60_000 + 1), (10, 271, 28));
        // 3 hours: the interval grows so the sheet stays below 600 frames.
        let (i, n, r) = grid(3 * 3_600_000);
        assert_eq!(i, 18);
        assert!(n <= MAX_FRAMES && r == (n + COLUMNS - 1) / COLUMNS);
        assert_eq!(grid(61_000), (10, 7, 1));
    }
}
