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

//! Signing in from an app: with username and password (accounts without two-factor sign-in),
//! or with Quick Connect, where the app shows a code that the user confirms in FreeX Web
//! (`POST /api/me/quick-connect`) or in another app that is signed in. Either way the app gets
//! a normal FreeX session, listed with the app's name under the user's sessions.

use super::{Cx, Out, json_out, user_guid};
use crate::AppState;
use crate::auth::now_ms;
use crate::db::{LoginEvent, User};
use crate::http::{ApiError, ApiResult, Ctx, JsonBody, ok};
use axum::Extension;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use rand::Rng;
use serde_json::{Value, json};
use std::sync::Arc;

/// A Quick Connect code is valid this long.
const QUICK_CONNECT_TTL_MS: i64 = 10 * 60_000;
/// Pending codes in total and from one address.
const QUICK_CONNECT_MAX: usize = 100;
const QUICK_CONNECT_PER_IP: usize = 5;

/// A pending Quick Connect.
#[derive(Clone, Debug)]
pub struct QuickConnect {
    code: String,
    device_id: String,
    device: String,
    app: String,
    version: String,
    ip: String,
    created: i64,
    /// Set when a signed-in user confirmed the code.
    user_id: Option<String>,
}

impl QuickConnect {
    fn to_json(&self, secret: &str) -> Value {
        json!({
            "Authenticated": self.user_id.is_some(),
            "Secret": secret,
            "Code": self.code,
            "DeviceId": self.device_id,
            "DeviceName": self.device,
            "AppName": self.app,
            "AppVersion": self.version,
            "DateAdded": super::dto::iso(self.created),
        })
    }

    fn label(&self) -> String {
        let mut s = if self.app.is_empty() { "Jellyfin app".to_string() } else { self.app.clone() };
        if !self.device.is_empty() {
            s = format!("{s} ({})", self.device);
        }
        s
    }
}

fn event(ctx: &Ctx, success: bool, detail: &str) -> LoginEvent {
    LoginEvent { time: now_ms(), ip: ctx.ip.clone(), user_agent: ctx.user_agent.clone(), success, detail: detail.into() }
}

/// The account as `UserDto`. Apps never see the account as admin: the server is administered
/// in FreeX Web, and the admin screens of the apps would find nothing to talk to.
pub fn user_dto(cx: &Cx, u: &User) -> Value {
    let mode = match u.subtitle_mode.as_str() {
        "off" => "None",
        "forced" => "OnlyForced",
        "foreign" => "Smart",
        "always" => "Always",
        _ => "Default",
    };
    let configuration = json!({
        "AudioLanguagePreference": u.audio_language,
        "PlayDefaultAudioTrack": u.audio_language.is_empty(),
        "SubtitleLanguagePreference": u.subtitle_language,
        "DisplayMissingEpisodes": false,
        "GroupedFolders": [],
        "SubtitleMode": mode,
        "DisplayCollectionsView": false,
        "EnableLocalPassword": false,
        "OrderedViews": [],
        "LatestItemsExcludes": [],
        "MyMediaExcludes": [],
        "HidePlayedInLatest": true,
        "RememberAudioSelections": true,
        "RememberSubtitleSelections": true,
        "EnableNextEpisodeAutoPlay": true,
    });
    let transcoding = cx.app.transcoder.is_some();
    let policy: serde_json::Map<String, Value> = [
        ("IsAdministrator", json!(false)),
        ("IsHidden", json!(false)),
        ("EnableCollectionManagement", json!(false)),
        ("EnableSubtitleManagement", json!(false)),
        ("EnableLyricManagement", json!(false)),
        ("IsDisabled", json!(false)),
        ("BlockedTags", json!([])),
        ("AllowedTags", json!([])),
        ("EnableUserPreferenceAccess", json!(false)),
        ("AccessSchedules", json!([])),
        ("BlockUnratedItems", json!([])),
        ("EnableRemoteControlOfOtherUsers", json!(false)),
        ("EnableSharedDeviceControl", json!(false)),
        ("EnableRemoteAccess", json!(true)),
        ("EnableLiveTvManagement", json!(false)),
        ("EnableLiveTvAccess", json!(false)),
        ("EnableMediaPlayback", json!(true)),
        ("EnableAudioPlaybackTranscoding", json!(transcoding)),
        ("EnableVideoPlaybackTranscoding", json!(transcoding)),
        ("EnablePlaybackRemuxing", json!(transcoding)),
        ("ForceRemoteSourceTranscoding", json!(false)),
        ("EnableContentDeletion", json!(false)),
        ("EnableContentDeletionFromFolders", json!([])),
        ("EnableContentDownloading", json!(true)),
        ("EnableSyncTranscoding", json!(false)),
        ("EnableMediaConversion", json!(false)),
        ("EnabledDevices", json!([])),
        ("EnableAllDevices", json!(true)),
        ("EnabledChannels", json!([])),
        ("EnableAllChannels", json!(false)),
        ("EnabledFolders", json!([])),
        ("EnableAllFolders", json!(true)),
        ("InvalidLoginAttemptCount", json!(0)),
        ("LoginAttemptsBeforeLockout", json!(-1)),
        ("MaxActiveSessions", json!(0)),
        ("EnablePublicSharing", json!(false)),
        ("BlockedMediaFolders", json!([])),
        ("BlockedChannels", json!([])),
        ("RemoteClientBitrateLimit", json!(0)),
        ("AuthenticationProviderId", json!("FreeX")),
        ("PasswordResetProviderId", json!("FreeX")),
        ("SyncPlayAccess", json!("None")),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let mut m = json!({
        "Name": u.username,
        "ServerId": cx.jf().server_id,
        "ServerName": cx.app.config.server.name,
        "Id": user_guid(&u.id),
        "HasPassword": true,
        "HasConfiguredPassword": true,
        "HasConfiguredEasyPassword": false,
        "EnableAutoLogin": false,
        "LastLoginDate": super::dto::iso(now_ms()),
        "LastActivityDate": super::dto::iso(now_ms()),
    });
    m["Configuration"] = configuration;
    m["Policy"] = Value::Object(policy);
    m
}

/// Starts a FreeX session for the app and answers with `AuthenticationResult`.
fn signed_in(cx: &Cx, u: &User, method: &str) -> Out {
    let label = cx.req.client.label();
    let token = cx.app.db.create_session(&u.id, &cx.ctx.ip, &cx.ctx.user_agent);
    cx.app.db.add_login_event(&u.id, &event(&cx.ctx, true, method));
    tracing::info!("Login '{}' from {} with {label} ({method})", u.username, cx.ctx.ip);
    let c = &cx.req.client;
    let now = super::dto::iso(now_ms());
    let session = json!({
        "Id": crate::db::session_id(&token),
        "UserId": user_guid(&u.id),
        "UserName": u.username,
        "Client": c.app,
        "DeviceName": c.device,
        "DeviceId": c.device_id,
        "ApplicationVersion": c.version,
        "RemoteEndPoint": cx.ctx.ip,
        "ServerId": cx.jf().server_id,
        "LastActivityDate": now,
        "LastPlaybackCheckIn": now,
        "PlayableMediaTypes": ["Video"],
        "IsActive": true,
        "SupportsMediaControl": false,
        "SupportsRemoteControl": false,
        "HasCustomDeviceName": false,
        "SupportedCommands": [],
        "AdditionalUsers": [],
        "NowPlayingQueue": [],
        "NowPlayingQueueFullItems": [],
    });
    json_out(json!({"User": user_dto(cx, u), "SessionInfo": session, "AccessToken": token, "ServerId": cx.jf().server_id}))
}

/// `POST /Users/AuthenticateByName` with `{Username, Pw}`. Accounts with two-factor sign-in
/// cannot sign in this way (the apps know no second factor): they use Quick Connect.
pub async fn by_name(cx: &Cx) -> Out {
    let username = cx.req.field("Username").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let password = cx.req.field("Pw").or_else(|| cx.req.field("Password")).and_then(Value::as_str).unwrap_or("").to_string();
    let app = &cx.app;
    let wait = app.guard.retry_after(&cx.ctx.ip, &username);
    if wait > 0 {
        let mut r = StatusCode::TOO_MANY_REQUESTS.into_response();
        r.headers_mut().insert(axum::http::header::RETRY_AFTER, wait.into());
        return Err(r.into());
    }
    let user = app.db.user_by_name(&username);
    // Unknown accounts take as long as known ones, so their existence does not show.
    let hash = user.as_ref().map(|u| u.password_hash.clone()).unwrap_or_else(|| app.dummy_hash.clone());
    let password_ok = tokio::task::spawn_blocking(move || crate::auth::verify_password(&password, &hash)).await.unwrap_or(false);
    let Some(u) = user.filter(|_| password_ok) else {
        app.guard.failure(&cx.ctx.ip, &username);
        if let Some(u) = app.db.user_by_name(&username) {
            app.db.add_login_event(&u.id, &event(&cx.ctx, false, "wrong-password"));
        }
        tracing::warn!("Failed login for '{}' from {} (Jellyfin app)", username, cx.ctx.ip);
        return Err(StatusCode::UNAUTHORIZED.into());
    };
    if u.totp_enabled {
        // The password was right, but this way would skip the second factor.
        tracing::info!("'{}' has two-factor sign-in; the Jellyfin app {} has to use Quick Connect", u.username, cx.req.client.label());
        return Err(StatusCode::UNAUTHORIZED.into());
    }
    app.guard.success(&cx.ctx.ip, &username);
    signed_in(cx, &u, "jellyfin")
}

/// `POST /QuickConnect/Initiate`: a new code for the app to show.
pub fn initiate(cx: &Cx) -> Out {
    let now = now_ms();
    let mut all = cx.jf().quick.lock().unwrap();
    all.retain(|_, q| now - q.created < QUICK_CONNECT_TTL_MS);
    if all.len() >= QUICK_CONNECT_MAX || all.values().filter(|q| q.ip == cx.ctx.ip).count() >= QUICK_CONNECT_PER_IP {
        return Err(StatusCode::TOO_MANY_REQUESTS.into());
    }
    let code = loop {
        let c = format!("{:06}", rand::thread_rng().gen_range(0..1_000_000));
        if !all.values().any(|q| q.code == c) {
            break c;
        }
    };
    let secret = crate::auth::sha256(&crate::auth::token());
    let c = &cx.req.client;
    let q = QuickConnect {
        code,
        device_id: c.device_id.clone(),
        device: c.device.clone(),
        app: c.app.clone(),
        version: c.version.clone(),
        ip: cx.ctx.ip.clone(),
        created: now,
        user_id: None,
    };
    let v = q.to_json(&secret);
    all.insert(secret, q);
    json_out(v)
}

/// `GET /QuickConnect/Connect?secret=`: whether the code was confirmed yet.
pub fn state(cx: &Cx) -> Out {
    let secret = cx.req.param("secret").unwrap_or_default();
    let all = cx.jf().quick.lock().unwrap();
    match all.get(&secret).filter(|q| now_ms() - q.created < QUICK_CONNECT_TTL_MS) {
        Some(q) => json_out(q.to_json(&secret)),
        None => Err(StatusCode::NOT_FOUND.into()),
    }
}

/// `POST /Users/AuthenticateWithQuickConnect` with `{Secret}`: signs in once the code is confirmed.
pub fn with_quick_connect(cx: &Cx) -> Out {
    let secret = cx.req.field("Secret").and_then(Value::as_str).unwrap_or("").to_string();
    let user_id = {
        let mut all = cx.jf().quick.lock().unwrap();
        match all.get(&secret) {
            Some(q) if now_ms() - q.created < QUICK_CONNECT_TTL_MS && q.user_id.is_some() => {
                all.remove(&secret).and_then(|q| q.user_id).expect("checked")
            }
            _ => return Err(StatusCode::UNAUTHORIZED.into()),
        }
    };
    let u = cx.app.db.user(&user_id).ok_or(super::Fail::from(StatusCode::UNAUTHORIZED))?;
    signed_in(cx, &u, "quick-connect")
}

/// Confirms a code for `user`; the label of the waiting app, or None for an unknown code.
fn confirm(app: &AppState, user: &User, code: &str) -> Option<String> {
    let code: String = code.chars().filter(char::is_ascii_digit).collect();
    let now = now_ms();
    let mut all = app.jellyfin.quick.lock().unwrap();
    let q = all.values_mut().find(|q| q.code == code && q.user_id.is_none() && now - q.created < QUICK_CONNECT_TTL_MS)?;
    q.user_id = Some(user.id.clone());
    tracing::info!("Quick Connect for '{}' confirmed: {}", user.username, q.label());
    Some(q.label())
}

/// `POST /QuickConnect/Authorize?code=` from an app that is signed in.
pub fn authorize(cx: &Cx) -> Out {
    let u = cx.user()?;
    let code = cx.req.param("code").unwrap_or_default();
    match confirm(&cx.app, u, &code) {
        Some(_) => json_out(json!(true)),
        None => Err(StatusCode::NOT_FOUND.into()),
    }
}

/// `POST /api/me/quick-connect` with `{code}`: FreeX Web confirms the code an app shows.
pub async fn authorize_quick_connect(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    let u = ctx.user()?;
    if !app.config.jellyfin.enabled {
        return Err(ApiError::not_found());
    }
    match confirm(&app, u, body.str("code").unwrap_or("")) {
        Some(device) => ok(json!({"device": device})),
        None => Err(ApiError::new(StatusCode::NOT_FOUND, "error.quickConnectCode")),
    }
}
