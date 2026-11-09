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

//! Intro and end-credit markers of episodes ("Skip intro"), found by comparing the sound of the
//! episodes of a season: what the beginnings have in common is the intro, what the ends have in
//! common are the credits. Runs in the background after scans; one season at a time.

pub mod fingerprint;

use crate::AppState;
use crate::auth::now_ms;
use crate::db::Db;
use crate::media::{Item, Kind};
use fingerprint::{FRAME_SECONDS, Fingerprint, Match};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

/// How much of the beginning is searched for the intro.
const INTRO_WINDOW: f64 = 600.0;
/// How much of the end is searched for credits.
const CREDITS_WINDOW: f64 = 360.0;
const MIN_SEGMENT: f64 = 15.0;
const MAX_INTRO: f64 = 150.0;
const MAX_BIT_ERRORS: u32 = 10;
const MAX_GAP: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
}

/// Markers of one episode; `file_modified` ties them to the analysed file.
#[derive(Clone, Copy, Debug)]
pub struct Markers {
    pub file_modified: i64,
    pub intro: Option<Segment>,
    pub credits: Option<Segment>,
    pub analyzed_at: i64,
}

pub struct Service {
    ffmpeg: String,
    markers: RwLock<HashMap<String, Markers>>,
    queued: Mutex<HashSet<String>>,
    serial: tokio::sync::Mutex<()>,
}

impl Service {
    pub fn new(db: &Db, ffmpeg: String) -> Service {
        Service { ffmpeg, markers: RwLock::new(db.markers()), queued: Mutex::default(), serial: tokio::sync::Mutex::new(()) }
    }

    /// The markers of an episode, if they belong to its current file.
    pub fn get(&self, episode: &Item) -> Option<Markers> {
        let m = *self.markers.read().unwrap().get(&episode.id)?;
        (episode.file.as_ref()?.modified == m.file_modified).then_some(m)
    }

    pub fn busy(&self) -> bool {
        !self.queued.lock().unwrap().is_empty()
    }

    /// Queues analysis of all seasons of a show library that have new or changed episodes.
    pub fn analyze_library(app: &Arc<AppState>, library_id: &str, force: bool) {
        let Some(service) = &app.markers else { return };
        if !service.queued.lock().unwrap().insert(library_id.to_string()) {
            return;
        }
        let (app, id) = (app.clone(), library_id.to_string());
        tokio::spawn(async move {
            let service = app.markers.as_ref().expect("checked");
            {
                let _turn = service.serial.lock().await;
                if let Some(lib) = app.db.library(&id).filter(|l| l.kind == "shows") {
                    let idx = app.catalog.snapshot();
                    let mut seasons = 0;
                    for show in idx.top_level(&lib.id) {
                        for season in idx.children(&show.id) {
                            let episodes: Vec<Arc<Item>> = idx.children(&season.id).to_vec();
                            if service.analyze_season(&app.db, &episodes, force).await {
                                seasons += 1;
                            }
                        }
                    }
                    if seasons > 0 {
                        tracing::info!("Intro/credits detection: {seasons} seasons analysed in '{}'", lib.name);
                    }
                }
            }
            service.queued.lock().unwrap().remove(&id);
        });
    }

    async fn analyze_season(&self, db: &Db, episodes: &[Arc<Item>], force: bool) -> bool {
        let eps: Vec<&Arc<Item>> = episodes
            .iter()
            .filter(|e| e.kind == Kind::Episode && e.file.as_ref().is_some_and(|f| f.duration_ms > 120_000 && !f.audio_streams.is_empty()))
            .collect();
        if eps.len() < 2 || !(force || eps.iter().any(|e| self.get(e).is_none())) {
            return false;
        }
        let mut heads = Vec::new();
        let mut tails = Vec::new();
        for ep in &eps {
            let f = ep.file.as_ref().expect("filtered");
            let dur = f.duration_ms as f64 / 1000.0;
            let head = INTRO_WINDOW.min(dur * 0.4);
            let tail = CREDITS_WINDOW.min(dur * 0.3);
            let path = Path::new(&f.path);
            let (h, t) =
                tokio::join!(Fingerprint::of(&self.ffmpeg, path, 0.0, head), Fingerprint::of(&self.ffmpeg, path, dur - tail, tail));
            let (Some(h), Some(t)) = (h, t) else {
                tracing::debug!("Cannot read the sound of {}", f.path);
                return false;
            };
            heads.push(h);
            tails.push(t);
        }
        let min_frames = (MIN_SEGMENT / FRAME_SECONDS).ceil() as usize;
        let max_intro = (MAX_INTRO / FRAME_SECONDS).ceil() as usize;
        let found = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            for i in 0..heads.len() {
                let intro = best(&heads, i, min_frames, max_intro);
                let credits = best(&tails, i, min_frames, usize::MAX);
                out.push((intro, credits));
            }
            out
        })
        .await
        .unwrap_or_default();
        let now = now_ms();
        let mut all = self.markers.write().unwrap();
        for (ep, (intro, credits)) in eps.iter().zip(found) {
            let f = ep.file.as_ref().expect("filtered");
            let clamp = |s: Option<Segment>, credits: bool| {
                s.map(|mut s| {
                    // Credits run until the end of the file, even if the music stops a bit earlier.
                    if credits && f.duration_ms - s.end_ms < 20_000 {
                        s.end_ms = f.duration_ms;
                    }
                    s.end_ms = s.end_ms.min(f.duration_ms);
                    s
                })
            };
            let m = Markers { file_modified: f.modified, intro: clamp(intro, false), credits: clamp(credits, true), analyzed_at: now };
            db.save_markers(&ep.id, &m);
            all.insert(ep.id.clone(), m);
        }
        true
    }
}

/// Compares fingerprint `i` with its neighbours in the season and returns the longest common
/// stretch as a segment of episode `i`.
fn best(fps: &[Fingerprint], i: usize, min_frames: usize, max_frames: usize) -> Option<Segment> {
    let a = &fps[i];
    // With two or more matches only the part common to all of them is used: a stretch that
    // happens to sound alike in just one pair (similar music under a scene) must not be skipped.
    let mut start: Option<usize> = None;
    let mut end: Option<usize> = None;
    let mut found = 0;
    let mut longest: Option<Match> = None;
    let neighbours = [i as i64 + 1, i as i64 - 1, i as i64 + 2, i as i64 - 2];
    for j in neighbours {
        if j < 0 || j as usize >= fps.len() || found >= 2 {
            continue;
        }
        let Some(m) = fingerprint::longest_common(a, &fps[j as usize], min_frames, MAX_BIT_ERRORS, MAX_GAP) else { continue };
        if m.length() > max_frames {
            continue;
        }
        found += 1;
        if longest.is_none_or(|l| m.length() > l.length()) {
            longest = Some(m);
        }
        start = Some(start.map_or(m.start_a, |s| s.max(m.start_a)));
        end = Some(end.map_or(m.end_a, |e| e.min(m.end_a)));
    }
    let (mut s, mut e) = (start?, end?);
    if found >= 2 && e.saturating_sub(s) < min_frames {
        // The matches disagree; fall back to the longest one.
        let l = longest?;
        (s, e) = (l.start_a, l.end_a);
    }
    let to_ms = |frame: usize| ((a.start_seconds + frame as f64 * FRAME_SECONDS) * 1000.0).round() as i64;
    Some(Segment { start_ms: to_ms(s), end_ms: to_ms(e) })
}
