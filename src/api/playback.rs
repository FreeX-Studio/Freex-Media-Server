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

//! Playback: the playback decision, the video file itself, subtitles, watch progress, the
//! watched flag and the watch history.

use super::media::{View, visible_item};
use crate::AppState;
use crate::activity::Start;
use crate::auth::now_ms;
use crate::db::{Progress, User};
use crate::http::{ApiError, ApiResult, Ctx, JsonBody, no_content, ok};
use crate::media::catalog::Index;
use crate::media::playback::{self, Capabilities, Mode, QUALITIES};
use crate::media::{Item, Kind};
use crate::transcode::{Session, StartError};
use crate::webhooks::Webhooks;
use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Positions before this are not worth resuming.
const MIN_RESUME_MS: i64 = 30_000;
/// From this share of the length on, an item counts as watched.
const WATCHED_THRESHOLD: f64 = 0.90;

/// A movie or episode the user may see that has a file.
pub(super) fn playable(idx: &Index, user: &User, id: &str) -> Result<Arc<Item>, ApiError> {
    let it = visible_item(idx, user, id)?;
    if !it.kind.playable() || it.file.is_none() {
        return Err(ApiError::bad_request("error.notPlayable"));
    }
    Ok(it)
}

pub(super) fn num(body: &JsonBody, key: &str) -> Option<i64> {
    body.get(key).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
}

/// Whether a subtitle id (`x0` for a file next to the video, `e3` for a stream inside it) names
/// a text subtitle.
fn is_text_subtitle(f: &crate::media::MediaFile, sid: &str) -> bool {
    if let Some(n) = sid.strip_prefix('x') {
        return n.parse::<usize>().is_ok_and(|i| i < f.subtitles.len());
    }
    sid.strip_prefix('e')
        .and_then(|n| n.parse::<i32>().ok())
        .is_some_and(|n| f.subtitle_streams.iter().any(|s| s.stream_index == n && s.text_based()))
}

/// Decides how to play an item in this player. Body: `{caps, quality, audio, subtitle, startMs}`;
/// with `"player": "external"` (mpv in FreeX Desktop, which decodes everything) the file is always
/// played directly; with `"tokenUrls": true` the URL carries its own access token (FreeX TV).
pub async fn start(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>, body: JsonBody) -> ApiResult {
    let u = ctx.user()?;
    let it = playable(&app.catalog.snapshot(), u, &id)?;
    let f = it.file.as_ref().expect("playable");
    let caps = Capabilities::parse(body.get("caps").unwrap_or(&Value::Null));
    // "auto" (or nothing): the user's default for this network, automatic from the bandwidth the
    // player measured.
    let home = playback::home_network(&ctx.ip);
    let saved = if home { &u.quality_home } else { &u.quality_away };
    let asked = body.str("quality").filter(|q| playback::known_quality(q)).unwrap_or("auto");
    let wanted = if asked == "auto" { saved.as_str() } else { asked };
    let automatic = wanted == "auto" || !playback::known_quality(wanted);
    let quality =
        if automatic { playback::automatic(home, num(&body, "bandwidth").unwrap_or(0), f.bitrate) } else { playback::quality(wanted) };
    let audio = num(&body, "audio").unwrap_or(-1) as i32;
    let mut start = num(&body, "startMs").unwrap_or(0).max(0);
    if f.duration_ms > 0 && start >= f.duration_ms {
        start = 0;
    }
    let plan = if body.str("player") == Some("external") {
        playback::external(f, audio)
    } else {
        playback::plan(f, &caps, quality, audio, body.str("subtitle"), app.transcoder.is_some())
    };
    // Text subtitles burned into the picture, on request.
    let burnable = app.tools.burn_text && app.transcoder.is_some();
    let plan = match body.str("subtitle").filter(|_| burnable && body.bool("burnSubtitle") == Some(true)) {
        Some(sid) if is_text_subtitle(f, sid) => match app.subtitles.vtt(&it, sid).await {
            Some(vtt) => playback::burn_text(f, plan, quality, vtt),
            None => plan,
        },
        _ => plan,
    };
    let mut m = json!({
        "mode": plan.mode.name(),
        "reason": ctx.t(&plan.reason),
        "audio": plan.audio_index,
        "burnedSubtitle": if plan.burn_text.is_some() {
            body.str("subtitle").map(str::to_string)
        } else {
            (plan.burn_subtitle >= 0).then(|| format!("e{}", plan.burn_subtitle))
        },
        "textBurnIn": burnable,
        "durationMs": f.duration_ms,
        "qualities": QUALITIES.iter().map(|q| json!({"id": q.id, "label": q.label})).collect::<Vec<_>>(),
        "quality": quality.id,
        "automatic": automatic,
        "network": if home { "home" } else { "away" },
        "startMs": start,
    });
    match (plan.mode, &app.transcoder) {
        (Mode::Direct, _) | (_, None) => m["url"] = json!(format!("/api/items/{}/stream", it.id)),
        (_, Some(t)) => match t.start(&u.id, &it, &plan, start).await {
            Ok(s) => {
                m["session"] = json!(s.id);
                m["url"] = json!(format!("/api/transcode/{}/index.m3u8", s.id));
            }
            Err(StartError::Busy) => return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "error.transcoderBusy")),
            Err(StartError::Failed(why)) => {
                tracing::warn!("Conversion of '{}' failed to start: {why}", it.title);
                return Err(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "error.transcodeFailedWith").arg(why));
            }
        },
    }
    if body.bool("tokenUrls") == Some(true) {
        // For native players that cannot send the token (FreeX TV): the same files under URLs with a token.
        let session = m["session"].as_str().map(str::to_string);
        let token = app.cast_grants.grant(&u.id, &it.id, session.clone(), f.duration_ms);
        let file = if session.is_some() { "hls/index.m3u8" } else { "stream" };
        m["url"] = json!(format!("{}{token}/{file}", super::extras::CAST_PREFIX));
    }
    let bitrate = playback::stream_bitrate(f, &plan);
    let mode = plan.mode.name();
    let fresh = app.activity.started(Start {
        user_id: &u.id,
        item_id: &it.id,
        client: body.str("client"),
        device: body.str("device").unwrap_or(""),
        mode: plan.mode.name(),
        reason: plan.reason,
        quality: quality.label,
        session: m["session"].as_str().map(str::to_string),
        offset_ms: start,
        ip: &ctx.ip,
        user_agent: &ctx.user_agent,
        bitrate,
    });
    if fresh {
        let idx = app.catalog.snapshot();
        let device = body.str("device").unwrap_or("");
        Webhooks::emit(&app, "play", playback_event(&idx, u, &it, device, &ctx.ip, start, Some(mode)));
    }
    ok(m)
}

/// The body of a playback webhook.
pub(super) fn playback_event(idx: &Index, u: &User, it: &Item, device: &str, ip: &str, offset_ms: i64, mode: Option<&str>) -> Value {
    json!({
        "user": crate::webhooks::user_json(u),
        "item": crate::webhooks::item_json(idx, it),
        "player": {"device": device, "ip": ip, "mode": mode},
        "offsetMs": offset_ms,
    })
}

/// A conversion session of the signed-in user; 404 for anyone else.
fn own_session(app: &AppState, user: &User, id: &str) -> Result<Arc<Session>, ApiError> {
    app.transcoder.as_ref().and_then(|t| t.session(id)).filter(|s| s.user_id == user.id).ok_or_else(ApiError::not_found)
}

/// The playlist, the init segment or a segment of a conversion. Segments are converted on
/// demand, so this can take a few seconds, longer after a far jump.
pub async fn transcode_file(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path((sid, name)): Path<(String, String)>,
) -> ApiResult {
    let u = ctx.user()?;
    let s = own_session(&app, u, &sid)?;
    send_transcode_file(&s, &name).await
}

/// The playlist, the init segment or a media segment of a session.
pub(super) async fn send_transcode_file(s: &Session, name: &str) -> ApiResult {
    if name == "index.m3u8" {
        return Ok(
            ([(header::CONTENT_TYPE, "application/vnd.apple.mpegurl"), (header::CACHE_CONTROL, "no-cache")], s.playlist()).into_response()
        );
    }
    let parts = if name == "init.mp4" {
        s.init(Duration::from_secs(30)).await.map(|p| vec![p])
    } else {
        let index = name.strip_prefix('s').and_then(|n| n.strip_suffix(".m4s")).filter(|n| n.len() <= 6).and_then(|n| n.parse().ok());
        let Some(index) = index else { return Err(ApiError::not_found()) };
        s.segment(index, Duration::from_secs(60)).await
    };
    let Some(parts) = parts else {
        let log = s.log();
        let last = log.lines().last().unwrap_or("").to_string();
        let status = if name == "init.mp4" { StatusCode::INTERNAL_SERVER_ERROR } else { StatusCode::NOT_FOUND };
        return Err(if last.is_empty() {
            ApiError::new(status, "error.transcodeFailed")
        } else {
            ApiError::new(status, "error.transcodeFailedWith").arg(last)
        });
    };
    // A segment is several fragments of ffmpeg, sent one after the other.
    let mut body = Vec::new();
    for p in parts {
        body.extend(tokio::fs::read(&p).await.map_err(|_| ApiError::not_found())?);
    }
    let kind = if name.ends_with(".mp4") { "video/mp4" } else { "video/iso.segment" };
    Ok(([(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, "private, max-age=3600")], body).into_response())
}

pub async fn stop_transcode(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(sid): Path<String>) -> ApiResult {
    let u = ctx.user()?;
    let s = own_session(&app, u, &sid)?;
    if let Some(t) = &app.transcoder {
        t.stop(&s.id);
    }
    no_content()
}

/// The video file, with byte ranges for seeking; `?download=1` as attachment.
pub async fn stream(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> ApiResult {
    let u = ctx.user()?;
    let it = playable(&app.catalog.snapshot(), u, &id)?;
    let f = it.file.as_ref().expect("playable");
    let path = std::path::PathBuf::from(&f.path);
    if !path.is_file() {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "error.fileGone"));
    }
    let mut r = crate::files::send(&path, 0, &headers).await?;
    if q.get("download").is_some_and(|v| v == "1") {
        let name: String = f.file_name().chars().map(|c| if matches!(c, '"' | '\\' | '\r' | '\n') { '_' } else { c }).collect();
        // ASCII fallback plus the exact name (RFC 6266) for names with umlauts.
        let ascii: String = name.chars().map(|c| if c.is_ascii() && !c.is_control() { c } else { '_' }).collect();
        let encoded: String = name
            .bytes()
            .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
            .collect();
        if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")) {
            r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Ok(r)
}

/// Subtitles, always as WebVTT; the player renders them itself.
pub async fn subtitle(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path((id, sid)): Path<(String, String)>,
) -> ApiResult {
    let u = ctx.user()?;
    let it = playable(&app.catalog.snapshot(), u, &id)?;
    let sid = sid.strip_suffix(".vtt").unwrap_or(&sid);
    let text = app.subtitles.vtt(&it, sid).await.ok_or_else(ApiError::not_found)?;
    Ok(([(header::CONTENT_TYPE, "text/vtt; charset=utf-8"), (header::CACHE_CONTROL, "private, max-age=3600")], text).into_response())
}

/// Position reports of the player. Near the end the item counts as watched (once per playthrough).
pub async fn progress(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
    body: JsonBody,
) -> ApiResult {
    let u = ctx.user()?;
    let idx = app.catalog.snapshot();
    let it = playable(&idx, u, &id)?;
    let offset = num(&body, "offsetMs").unwrap_or(0).max(0);
    let known = it.duration_ms();
    let duration = if known > 0 { known } else { num(&body, "durationMs").unwrap_or(0).max(0) };
    if known == 0 && duration > 0 {
        // ffprobe was not available: remember the length the player reported.
        let mut changed = Item::clone(&it);
        if let Some(f) = changed.file.as_mut() {
            f.duration_ms = duration;
        }
        app.catalog.update_item(&app.db, changed);
    }
    let change = app.activity.progress(
        &u.id,
        &it.id,
        body.str("client"),
        offset,
        body.str("state").unwrap_or("playing"),
        body.bool("stopped").unwrap_or(false),
    );
    if let Some(event) = change {
        Webhooks::emit(&app, event, playback_event(&idx, u, &it, body.str("device").unwrap_or(""), &ctx.ip, offset, None));
    }
    if it.kind == Kind::Extra {
        // Trailers and bonus videos keep no resume position and no history.
        return ok(Progress::default().to_json());
    }
    // A version of a movie shares the movie's watch state.
    let key = it.key().to_string();
    let old = app.db.progress(&u.id, &key);
    let now = now_ms();
    let p = if duration > 0 && offset as f64 >= duration as f64 * WATCHED_THRESHOLD {
        let new_view = !old.watched || old.offset_ms > 0;
        if new_view {
            record_history(&app, &idx, u, &it, now);
            Webhooks::emit(&app, "watched", playback_event(&idx, u, &it, body.str("device").unwrap_or(""), &ctx.ip, offset, None));
        }
        Progress { offset_ms: 0, watched: true, view_count: old.view_count + i64::from(new_view), updated_at: now }
    } else if offset < MIN_RESUME_MS {
        Progress { offset_ms: 0, ..old }
    } else {
        Progress { offset_ms: offset, updated_at: now, ..old }
    };
    app.db.set_progress(&u.id, &key, p);
    ok(p.to_json())
}

/// A finished playback goes into the history, with the title as it is shown now.
fn record_history(app: &AppState, idx: &Index, u: &User, it: &Item, now: i64) {
    let parent = if it.kind == Kind::Episode {
        let season = idx.parent(it).map_or(0, |s| s.index);
        let show = idx.show_of(it).map_or("", |s| s.title.as_str());
        let s = if season == 0 { "Special".to_string() } else { format!("S{season}") };
        format!("{show} · {s} E{}", it.index)
    } else if it.year > 0 {
        it.year.to_string()
    } else {
        String::new()
    };
    app.db.add_history(&u.id, it.key(), now, &it.title, &parent);
}

/// Marks an item (a movie, an episode, or all episodes of a season or show) as watched or not.
pub async fn watched(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    set_watched(&app, &ctx, &id, true)
}

pub async fn unwatched(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    set_watched(&app, &ctx, &id, false)
}

fn set_watched(app: &AppState, ctx: &Ctx, id: &str, watched: bool) -> ApiResult {
    let u = ctx.user()?;
    let idx = app.catalog.snapshot();
    let it = visible_item(&idx, u, id)?;
    let now = now_ms();
    let mut leaves = Vec::new();
    collect_leaves(&idx, &it, &mut leaves);
    for leaf in leaves {
        let old = app.db.progress(&u.id, leaf.key());
        let p = if watched {
            Progress { offset_ms: 0, watched: true, view_count: old.view_count.max(1), updated_at: now }
        } else {
            Progress { offset_ms: 0, watched: false, view_count: old.view_count, updated_at: now }
        };
        app.db.set_progress(&u.id, leaf.key(), p);
    }
    ok(View::new(app, &idx, u).decorate(&it, false))
}

fn collect_leaves(idx: &Index, it: &Arc<Item>, out: &mut Vec<Arc<Item>>) {
    if it.kind.playable() {
        out.push(it.clone());
    } else {
        for c in idx.children(&it.id) {
            collect_leaves(idx, c, out);
        }
    }
}

// ====================================================================== history

pub async fn my_history(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let u = ctx.user()?;
    ok(history(&app, u, Some(&u.id), &q))
}

/// All users (admins), or one with `?user=<id>`.
pub async fn all_history(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let u = ctx.admin()?;
    let user = q.get("user").filter(|s| !s.is_empty()).cloned();
    ok(history(&app, u, user.as_deref(), &q))
}

fn history(app: &AppState, viewer: &User, user_id: Option<&str>, q: &HashMap<String, String>) -> Value {
    let param = |k: &str, d: i64| q.get(k).and_then(|v| v.parse::<i64>().ok()).unwrap_or(d);
    let offset = param("offset", 0).max(0);
    let limit = param("limit", 50).clamp(1, 100);
    let mut entries = app.db.history(user_id, offset, limit + 1);
    let more = entries.len() as i64 > limit;
    entries.truncate(limit as usize);
    let idx = app.catalog.snapshot();
    let view = View::new(app, &idx, viewer);
    let mut users: HashMap<String, Value> = HashMap::new();
    let out: Vec<Value> = entries
        .iter()
        .map(|e| {
            // The item as it is now (title, artwork); null once it left the library.
            let item = idx.item(&e.item_id).filter(|it| view.sees(it)).map(|it| view.decorate(it, false));
            let mut m = json!({"id": e.id, "itemId": e.item_id, "watchedAt": e.watched_at, "title": e.title,
                               "parentTitle": e.parent_title, "item": item});
            if user_id != Some(viewer.id.as_str()) {
                let who = users.entry(e.user_id.clone()).or_insert_with(|| {
                    app.db.user(&e.user_id).map_or(Value::Null, |w| json!({"id": w.id, "username": w.username, "avatar": w.avatar_version}))
                });
                if !who.is_null() {
                    m["user"] = who.clone();
                }
            }
            m
        })
        .collect();
    json!({"entries": out, "more": more})
}

pub async fn remove_history(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(hid): Path<String>) -> ApiResult {
    let u = ctx.user()?;
    let id: i64 = hid.parse().map_err(|_| ApiError::not_found())?;
    if !app.db.remove_history(&u.id, id) {
        return Err(ApiError::not_found());
    }
    no_content()
}

pub async fn clear_history(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    let u = ctx.user()?;
    ok(json!({"removed": app.db.clear_history(&u.id)}))
}
