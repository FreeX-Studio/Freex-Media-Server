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

//! The server dashboard for admins: the load right now, what was watched (from the watch
//! history) and what the libraries hold.

use crate::AppState;
use crate::auth::now_ms;
use crate::http::{ApiResult, Ctx, ok};
use crate::media::{Item, Kind};
use axum::Extension;
use axum::extract::{Query, State};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

const DAY_MS: i64 = 86_400_000;

/// `?days=30&tz=-60`: the period of the statistics (1–365 days) and the browser's offset from
/// UTC in minutes (as `Date.getTimezoneOffset()` gives it), so that days end at local midnight.
pub async fn dashboard(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    ctx.admin()?;
    let days: i64 = q.get("days").and_then(|d| d.parse().ok()).unwrap_or(30).clamp(1, 365);
    let tz_ms = q.get("tz").and_then(|t| t.parse::<i64>().ok()).unwrap_or(0).clamp(-14 * 60, 14 * 60) * 60_000;
    ok(json!({
        "load": load(&app),
        "plays": plays(&app, days, tz_ms),
        "libraries": libraries(&app),
    }))
}

fn load(app: &AppState) -> Value {
    let l = app.system.load();
    let playbacks = app.activity.list();
    let playing: Vec<_> = playbacks.iter().filter(|p| p.state != "paused").collect();
    json!({
        "cpu": l.cpu,
        "serverCpu": l.server_cpu,
        "memory": l.memory,
        "cores": app.system.cores,
        "uptimeS": app.system.started.elapsed().as_secs(),
        "playbacks": playbacks.len(),
        "transcodes": app.transcoder.as_ref().map_or(0, |t| t.active_sessions()),
        "maxTranscodes": app.config.transcoder.max_sessions,
        "bandwidth": playing.iter().map(|p| p.bitrate).sum::<i64>(),
        "ffmpegCpu": l.processes.values().fold(0.0, |a, b| a + b),
    })
}

/// Plays per day, per user and per title over the last `days` days.
fn plays(app: &AppState, days: i64, tz_ms: i64) -> Value {
    let now = now_ms();
    // Local midnight `days - 1` days ago.
    let today = (now - tz_ms).div_euclid(DAY_MS) * DAY_MS + tz_ms;
    let since = today - (days - 1) * DAY_MS;
    let entries = app.db.history_since(since);
    let idx = app.catalog.snapshot();
    let duration = |id: &str| idx.item(id).map_or(0, |it| it.duration_ms());

    let mut per_day = vec![0usize; days as usize];
    let mut users: HashMap<&str, (usize, i64)> = HashMap::new();
    let mut titles: HashMap<String, (usize, String)> = HashMap::new();
    let mut total_ms = 0;
    for e in &entries {
        let d = ((e.watched_at - since) / DAY_MS) as usize;
        if let Some(slot) = per_day.get_mut(d) {
            *slot += 1;
        }
        let ms = duration(&e.item_id);
        total_ms += ms;
        let u = users.entry(&e.user_id).or_default();
        u.0 += 1;
        u.1 += ms;
        // Episodes count for their show.
        let (key, label) = match idx.item(&e.item_id).and_then(|it| show_of(&idx, it)) {
            Some(show) => (show.id.clone(), show.title.clone()),
            None => (e.item_id.clone(), e.title.clone()),
        };
        let t = titles.entry(key).or_insert((0, label));
        t.0 += 1;
    }
    let names: HashMap<String, String> = app.db.users().into_iter().map(|u| (u.id, u.username)).collect();
    let mut users: Vec<Value> = users
        .into_iter()
        .map(|(id, (n, ms))| json!({"id": id, "username": names.get(id).cloned().unwrap_or_else(|| "?".into()), "plays": n, "watchedMs": ms}))
        .collect();
    users.sort_by_key(|u| std::cmp::Reverse(u["plays"].as_u64().unwrap_or(0)));
    let mut titles: Vec<(String, usize, String)> = titles.into_iter().map(|(id, (n, label))| (id, n, label)).collect();
    titles.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.2.cmp(&b.2)));
    let titles: Vec<Value> = titles
        .into_iter()
        .take(10)
        .map(|(id, n, label)| json!({"id": idx.item(&id).map(|_| id.clone()), "title": label, "plays": n}))
        .collect();
    json!({
        "days": days,
        "since": since,
        "perDay": per_day,
        "total": entries.len(),
        "watchedMs": total_ms,
        "users": users,
        "titles": titles,
    })
}

fn show_of<'a>(idx: &'a crate::media::catalog::Index, it: &'a Item) -> Option<&'a Arc<Item>> {
    if it.kind == Kind::Episode { idx.show_of(it) } else { None }
}

/// What each library holds: entries, files, size, length, and how the files are made.
fn libraries(app: &AppState) -> Value {
    let idx = app.catalog.snapshot();
    let list: Vec<Value> = app
        .db
        .libraries()
        .iter()
        .map(|l| {
            let (mut movies, mut shows, mut episodes, mut files, mut size, mut duration, mut hdr) = (0, 0, 0, 0, 0i64, 0i64, 0);
            let mut resolutions: BTreeMap<String, usize> = BTreeMap::new();
            let mut codecs: BTreeMap<String, usize> = BTreeMap::new();
            for it in idx.of_library(&l.id) {
                match it.kind {
                    Kind::Movie => movies += 1,
                    Kind::Show => shows += 1,
                    Kind::Episode => episodes += 1,
                    _ => {}
                }
                // Every file: the versions of a movie, extras included.
                let files_of: Vec<&crate::media::MediaFile> =
                    if it.versions.is_empty() { it.file.iter().collect() } else { it.versions.iter().map(|v| &v.file).collect() };
                for f in files_of {
                    files += 1;
                    size += f.size;
                    if it.kind != Kind::Extra {
                        duration += f.duration_ms;
                    }
                    let r = f.resolution_label();
                    *resolutions.entry(if r.is_empty() { "?".into() } else { r.into() }).or_default() += 1;
                    let c = crate::media::playback::codec_kind(&f.video_codec);
                    *codecs.entry(if c.is_empty() { "?".into() } else { c }).or_default() += 1;
                    if f.hdr().is_some() {
                        hdr += 1;
                    }
                }
            }
            json!({
                "id": l.id, "name": l.name, "type": l.kind,
                "movies": movies, "shows": shows, "episodes": episodes, "files": files,
                "size": size, "durationMs": duration, "hdr": hdr,
                "resolutions": resolutions, "codecs": codecs,
            })
        })
        .collect();
    json!(list)
}
