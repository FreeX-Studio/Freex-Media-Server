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

//! Watch Together rooms, Chromecast playback and the intro/credits detection trigger.

use super::media::visible_item;
use super::playback::{num, playable, send_transcode_file};
use crate::AppState;
use crate::activity::Start;
use crate::auth::now_ms;
use crate::db::User;
use crate::http::{ApiError, ApiResult, Ctx, JsonBody, created, no_content, ok};
use crate::markers::Service as MarkerService;
use crate::media::catalog::Index;
use crate::media::playback::{self, Capabilities, Mode};
use crate::media::{Item, Kind};
use crate::together::{Action, UpdateError};
use crate::transcode::StartError;
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

// ====================================================================== markers

pub async fn analyze_markers(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    ctx.admin()?;
    let l = app.db.library(&id).ok_or_else(ApiError::not_found)?;
    MarkerService::analyze_library(&app, &l.id, true);
    let queued = app.markers.is_some() && l.kind == "shows";
    Ok((StatusCode::ACCEPTED, axum::Json(json!({"queued": queued}))).into_response())
}

// ====================================================================== watch together

fn room_item(app: &AppState, idx: &Index, user: &User, code: &str) -> Result<(), ApiError> {
    let item = app.together.item_of(code).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "error.roomGone"))?;
    // Only who may see the title can join; the invite link alone is not enough.
    visible_item(idx, user, &item).map(|_| ()).map_err(|_| ApiError::new(StatusCode::FORBIDDEN, "error.titleNotShared"))
}

pub async fn create_room(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    let u = ctx.user()?;
    let idx = app.catalog.snapshot();
    let it = visible_item(&idx, u, body.str("itemId").unwrap_or("")).map_err(|_| ApiError::bad_request("error.unknownItem"))?;
    if !it.kind.playable() {
        return Err(ApiError::bad_request("error.moviesAndEpisodesOnly"));
    }
    created(app.together.create(u, &it.id, num(&body, "positionMs").unwrap_or(0)))
}

pub async fn join_room(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(code): Path<String>) -> ApiResult {
    let u = ctx.user()?;
    room_item(&app, &app.catalog.snapshot(), u, &code)?;
    ok(app.together.join(&code, u).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "error.roomGone"))?)
}

pub async fn room_state(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(code): Path<String>,
    body: JsonBody,
) -> ApiResult {
    let u = ctx.user()?;
    let idx = app.catalog.snapshot();
    let item_id = body.str("itemId");
    if let Some(id) = item_id
        && visible_item(&idx, u, id).ok().filter(|it| it.kind.playable()).is_none()
    {
        return Err(ApiError::bad_request("error.unknownItem"));
    }
    let action = Action::parse(body.str("action").unwrap_or(""), item_id).map_err(ApiError::bad_request)?;
    match app.together.update(&code, u, action, num(&body, "positionMs").unwrap_or(0)) {
        None => Err(ApiError::new(StatusCode::NOT_FOUND, "error.roomGone")),
        Some(Err(UpdateError::NotMember)) => Err(ApiError::new(StatusCode::FORBIDDEN, "error.notInRoom")),
        Some(Ok(state)) => ok(state),
    }
}

pub async fn leave_room(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(code): Path<String>) -> ApiResult {
    let u = ctx.user()?;
    app.together.leave(&code, u);
    no_content()
}

/// The event stream of a subscription; ends when the room ends.
struct RoomEvents {
    sub: crate::together::Subscription,
}

impl futures_core::Stream for RoomEvents {
    type Item = Result<Event, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.sub.rx.poll_recv(cx) {
            Poll::Ready(Some((kind, state))) => Poll::Ready(Some(Ok(Event::default().event(kind).data(state.to_string())))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Server-sent events of a room; stays open until the browser closes it or the room ends.
pub async fn room_events(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(code): Path<String>) -> ApiResult {
    let u = ctx.user()?;
    room_item(&app, &app.catalog.snapshot(), u, &code)?;
    let sub = app.together.subscribe(&code, u).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "error.roomGone"))?;
    let sse = Sse::new(RoomEvents { sub }).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("ping"));
    Ok(([(header::CACHE_CONTROL, "no-cache"), (axum::http::HeaderName::from_static("x-accel-buffering"), "no")], sse).into_response())
}

// ====================================================================== chromecast

/// What the receiver can decode (first-generation Chromecasts included).
fn chromecast() -> Capabilities {
    let set = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
    Capabilities {
        containers: set(&["mp4"]),
        video: set(&["h264"]),
        audio: set(&["aac", "mp3"]),
        mse_video: set(&["h264"]),
        mse_audio: set(&["aac", "mp3"]),
        direct_play_allowed: true,
        hevc_main10: Some(false),
        mse_hevc_main10: Some(false),
    }
}

pub(super) const CAST_PREFIX: &str = "/api/cast/";
const EXTRA_VALIDITY_MS: i64 = 6 * 3_600_000;

/// Access of one Chromecast to one item, without a login: the receiver gets URLs with a token.
pub struct Grant {
    pub user_id: String,
    pub item_id: String,
    pub session: Option<String>,
    pub expires: i64,
}

#[derive(Default)]
pub struct Grants(std::sync::Mutex<HashMap<String, Grant>>);

impl Grants {
    fn get(&self, token: &str) -> Option<(String, String, Option<String>)> {
        let now = now_ms();
        let mut all = self.0.lock().unwrap();
        all.retain(|_, g| g.expires > now);
        all.get(token).map(|g| (g.user_id.clone(), g.item_id.clone(), g.session.clone()))
    }

    /// Lets a player that cannot send the sign-in token (a Chromecast, the native player of a TV)
    /// load the files of one playback, for the length of the title and a few hours more.
    pub(super) fn grant(&self, user_id: &str, item_id: &str, session: Option<String>, duration_ms: i64) -> String {
        let token = crate::auth::token();
        let validity = if duration_ms > 0 { duration_ms } else { 4 * 3_600_000 } + EXTRA_VALIDITY_MS;
        let grant = Grant { user_id: user_id.to_string(), item_id: item_id.to_string(), session, expires: now_ms() + validity };
        self.0.lock().unwrap().insert(token.clone(), grant);
        token
    }
}

/// Plans a cast. Body as for `/playback` (quality, audio, subtitle, startMs, client, device),
/// without capabilities. Answers with URLs relative to this server.
pub async fn start_cast(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
    body: JsonBody,
) -> ApiResult {
    let u = ctx.user()?;
    let it = playable(&app.catalog.snapshot(), u, &id)?;
    let f = it.file.as_ref().expect("playable");
    let quality = playback::quality(body.str("quality").unwrap_or("original"));
    let audio = num(&body, "audio").unwrap_or(-1) as i32;
    let mut start = num(&body, "startMs").unwrap_or(0).max(0);
    if f.duration_ms > 0 && start >= f.duration_ms {
        start = 0;
    }
    let plan = playback::plan(f, &chromecast(), quality, audio, body.str("subtitle"), app.transcoder.is_some());
    let token = crate::auth::token();
    let mut m = json!({
        "mode": plan.mode.name(),
        "reason": ctx.t(&plan.reason),
        "audio": plan.audio_index,
        "startMs": start,
        "durationMs": f.duration_ms,
        "image": format!("{CAST_PREFIX}{token}/image/{}", if it.kind == Kind::Episode { "thumb" } else { "poster" }),
    });
    let mut session = None;
    match (plan.mode, &app.transcoder) {
        (Mode::Direct, _) | (_, None) => {
            m["url"] = json!(format!("{CAST_PREFIX}{token}/stream"));
            m["contentType"] = json!("video/mp4");
            m["hls"] = json!(false);
        }
        (_, Some(t)) => match t.start(&u.id, &it, &plan, start).await {
            Ok(s) => {
                session = Some(s.id.clone());
                m["url"] = json!(format!("{CAST_PREFIX}{token}/hls/index.m3u8"));
                m["contentType"] = json!("application/x-mpegurl");
                m["hls"] = json!(true);
            }
            Err(StartError::Busy) => return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "error.transcoderBusy")),
            Err(StartError::Failed(why)) => {
                return Err(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "error.transcodeFailedWith").arg(why));
            }
        },
    }
    let burned = (plan.burn_subtitle >= 0).then(|| format!("e{}", plan.burn_subtitle));
    m["burnedSubtitle"] = json!(burned);
    // Subtitles the receiver can show as text track: files next to the video and text streams.
    let mut subs: Vec<Value> = Vec::new();
    let ids = (0..f.subtitles.len())
        .map(|i| format!("x{i}"))
        .chain(f.subtitle_streams.iter().filter(|s| s.text_based()).map(|s| format!("e{}", s.stream_index)));
    for sid in ids {
        if burned.as_deref() != Some(sid.as_str()) {
            subs.push(json!({"id": sid, "url": format!("{CAST_PREFIX}{token}/subtitles/{sid}.vtt")}));
        }
    }
    m["subtitles"] = json!(subs);
    let validity = if f.duration_ms > 0 { f.duration_ms } else { 4 * 3_600_000 } + EXTRA_VALIDITY_MS;
    app.cast_grants
        .0
        .lock()
        .unwrap()
        .insert(token, Grant { user_id: u.id.clone(), item_id: it.id.clone(), session: session.clone(), expires: now_ms() + validity });
    let bitrate = playback::stream_bitrate(f, &plan);
    let mode = plan.mode.name();
    let fresh = app.activity.started(Start {
        user_id: &u.id,
        item_id: &it.id,
        client: body.str("client"),
        device: body.str("device").unwrap_or("Chromecast"),
        mode: plan.mode.name(),
        reason: plan.reason,
        quality: quality.label,
        session,
        offset_ms: start,
        ip: &ctx.ip,
        user_agent: &ctx.user_agent,
        bitrate,
    });
    if fresh {
        let idx = app.catalog.snapshot();
        let ev = super::playback::playback_event(&idx, u, &it, body.str("device").unwrap_or("Chromecast"), &ctx.ip, start, Some(mode));
        crate::webhooks::Webhooks::emit(&app, "play", ev);
    }
    ok(m)
}

/// The item of a cast token; 404 if the token is unknown, expired or the user is gone.
fn cast_item(app: &AppState, token: &str) -> Result<(Arc<Item>, Option<String>), ApiError> {
    let (user_id, item_id, session) = app.cast_grants.get(token).ok_or_else(ApiError::not_found)?;
    if app.db.user(&user_id).is_none() {
        return Err(ApiError::not_found());
    }
    let it = app.catalog.snapshot().item(&item_id).cloned().ok_or_else(ApiError::not_found)?;
    if it.file.is_none() {
        return Err(ApiError::not_found());
    }
    Ok((it, session))
}

pub async fn cast_stream(State(app): State<Arc<AppState>>, Path(token): Path<String>, headers: HeaderMap) -> ApiResult {
    let (it, _) = cast_item(&app, &token)?;
    let path = std::path::PathBuf::from(&it.file.as_ref().expect("checked").path);
    if !path.is_file() {
        return Err(ApiError::not_found());
    }
    crate::files::send(&path, 0, &headers).await
}

pub async fn cast_hls(State(app): State<Arc<AppState>>, Path((token, name)): Path<(String, String)>) -> ApiResult {
    let (_, session) = cast_item(&app, &token)?;
    let s = session.and_then(|id| app.transcoder.as_ref()?.session(&id)).ok_or_else(ApiError::not_found)?;
    send_transcode_file(&s, &name).await
}

pub async fn cast_subtitle(State(app): State<Arc<AppState>>, Path((token, sid)): Path<(String, String)>) -> ApiResult {
    let (it, _) = cast_item(&app, &token)?;
    let sid = sid.strip_suffix(".vtt").unwrap_or(&sid);
    let text = app.subtitles.vtt(&it, sid).await.ok_or_else(ApiError::not_found)?;
    Ok(([(header::CONTENT_TYPE, "text/vtt; charset=utf-8")], text).into_response())
}

pub async fn cast_image(State(app): State<Arc<AppState>>, Path((token, kind)): Path<(String, String)>, headers: HeaderMap) -> ApiResult {
    let (it, _) = cast_item(&app, &token)?;
    let path = super::media::image_path(&app, &it, &kind).await.ok_or_else(ApiError::not_found)?;
    crate::files::send(&path, 3600, &headers).await
}
