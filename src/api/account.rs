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

//! Signing in and the own account: setup of the first admin, sign-in with optional second factor,
//! sessions, login history, password, two-factor setup and preferences.

use crate::AppState;
use crate::auth::{self, now_ms};
use crate::db::{LoginEvent, User, session_id};
use crate::http::{ApiError, ApiResult, Ctx, JsonBody, no_content, ok, session_cookie};
use crate::i18n::{self, Text};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A password check that is waiting for the second factor.
pub struct Challenge {
    user_id: String,
    expires: Instant,
    attempts: u32,
}

const CHALLENGE_TTL: Duration = Duration::from_secs(5 * 60);
const CHALLENGE_ATTEMPTS: u32 = 5;

/// Audio and subtitle languages users can prefer (ISO 639-1).
const MEDIA_LANGUAGES: [&str; 22] =
    ["de", "en", "fr", "es", "it", "nl", "pt", "pl", "ru", "tr", "sv", "da", "no", "fi", "cs", "hu", "el", "ja", "ko", "zh", "ar", "hi"];
pub const SUBTITLE_MODES: [&str; 4] = ["off", "forced", "foreign", "always"];

pub async fn setup(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    // One setup at a time, so two requests cannot both create the first admin.
    let _lock = app.setup_lock.lock().await;
    if app.db.has_users() {
        return Err(ApiError::new(StatusCode::CONFLICT, "error.alreadySetUp"));
    }
    let username = valid_username(body.str("username").unwrap_or(""))?;
    let password = valid_password(&app, body.str("password").unwrap_or(""), &username)?;
    let u = app
        .db
        .add_user(&username, &auth::hash_password(&password), true)
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "error.usernameTaken"))?;
    tracing::info!("Admin account '{}' created from {}", username, ctx.ip);
    start_session(&app, &ctx, &u, "password")
}

pub async fn login(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    let username = body.string("username").trim().to_string();
    let password = body.string("password");
    throttle(&app, &ctx, &username)?;
    let user = app.db.user_by_name(&username);
    // Unknown accounts take as long as known ones, so their existence does not show.
    let hash = user.as_ref().map(|u| u.password_hash.as_str()).unwrap_or(&app.dummy_hash);
    let password_ok = {
        let (p, h) = (password.clone(), hash.to_string());
        tokio::task::spawn_blocking(move || auth::verify_password(&p, &h)).await.unwrap_or(false)
    };
    let Some(u) = user.filter(|_| password_ok) else {
        app.guard.failure(&ctx.ip, &username);
        if let Some(u) = app.db.user_by_name(&username) {
            app.db.add_login_event(&u.id, &event(&ctx, false, "wrong-password"));
        }
        tracing::warn!("Failed login for '{}' from {}", username, ctx.ip);
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "error.wrongLogin"));
    };
    if u.totp_enabled {
        let challenge = auth::token();
        let mut c = app.challenges.lock().unwrap();
        c.retain(|_, ch| ch.expires > Instant::now());
        c.insert(auth::sha256(&challenge), Challenge { user_id: u.id.clone(), expires: Instant::now() + CHALLENGE_TTL, attempts: 0 });
        return ok(json!({"twoFactorRequired": true, "challenge": challenge}));
    }
    app.guard.success(&ctx.ip, &username);
    start_session(&app, &ctx, &u, "password")
}

pub async fn verify_second_factor(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    let key = auth::sha256(body.str("challenge").unwrap_or(""));
    let code = body.string("code").trim().to_string();
    let user_id = {
        let mut c = app.challenges.lock().unwrap();
        match c.get(&key) {
            Some(ch) if ch.expires > Instant::now() => ch.user_id.clone(),
            _ => {
                c.remove(&key);
                return Err(ApiError::new(StatusCode::UNAUTHORIZED, "error.challengeExpired"));
            }
        }
    };
    let mut u = app.db.user(&user_id).ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "error.wrongLogin"))?;
    throttle(&app, &ctx, &u.username)?;
    let mut method = None;
    if let Some(step) = u.totp_secret.as_deref().and_then(|s| auth::totp_verify(s, &code, u.totp_last_step)) {
        u.totp_last_step = step;
        method = Some("password+totp");
    } else if let Some(hash) = auth::recovery_code_hash(&code)
        && let Some(i) = u.recovery_codes.iter().position(|c| *c == hash)
    {
        u.recovery_codes.remove(i);
        method = Some("password+recovery");
    }
    let Some(method) = method else {
        app.guard.failure(&ctx.ip, &u.username);
        app.db.add_login_event(&u.id, &event(&ctx, false, "wrong-code"));
        tracing::warn!("Failed second factor for '{}' from {}", u.username, ctx.ip);
        let mut c = app.challenges.lock().unwrap();
        let attempts = c.get(&key).map(|ch| ch.attempts + 1).unwrap_or(CHALLENGE_ATTEMPTS);
        if attempts >= CHALLENGE_ATTEMPTS {
            c.remove(&key);
            return Err(ApiError::new(StatusCode::UNAUTHORIZED, "error.tooManyCodes"));
        }
        if let Some(ch) = c.get_mut(&key) {
            ch.attempts = attempts;
        }
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "error.wrongCode"));
    };
    app.db.update_user(&u);
    app.challenges.lock().unwrap().remove(&key);
    app.guard.success(&ctx.ip, &u.username);
    start_session(&app, &ctx, &u, method)
}

fn throttle(app: &AppState, ctx: &Ctx, account: &str) -> Result<(), ApiError> {
    let wait = app.guard.retry_after(&ctx.ip, account);
    if wait == 0 {
        return Ok(());
    }
    let time = if wait < 90 { Text::new("time.seconds").arg(wait as i64) } else { Text::new("time.minutes").arg(wait.div_ceil(60) as i64) };
    let mut e = ApiError::new(StatusCode::TOO_MANY_REQUESTS, "error.tooManyAttempts").arg(time);
    e.retry_after = Some(wait);
    Err(e)
}

fn event(ctx: &Ctx, success: bool, detail: &str) -> LoginEvent {
    LoginEvent { time: now_ms(), ip: ctx.ip.clone(), user_agent: ctx.user_agent.clone(), success, detail: detail.into() }
}

fn start_session(app: &AppState, ctx: &Ctx, u: &User, method: &str) -> ApiResult {
    let token = app.db.create_session(&u.id, &ctx.ip, &ctx.user_agent);
    app.db.add_login_event(&u.id, &event(ctx, true, method));
    tracing::info!("Login '{}' from {} ({})", u.username, ctx.ip, method);
    let mut r = Json(json!({"token": token, "user": user_json(u, ctx)})).into_response();
    session_cookie(&mut r, &token, i64::from(app.config.security.session_days) * 86_400, ctx.secure);
    Ok(r)
}

pub async fn logout(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    ctx.user()?;
    if let Some(t) = &ctx.token {
        app.db.remove_session(t);
    }
    let mut r = StatusCode::NO_CONTENT.into_response();
    session_cookie(&mut r, "", 0, ctx.secure);
    Ok(r)
}

pub async fn me(Extension(ctx): Extension<Ctx>) -> ApiResult {
    ok(user_json(ctx.user()?, &ctx))
}

/// The account with the network the request comes from (`home` or `away`), which decides the
/// default quality.
fn user_json(u: &User, ctx: &Ctx) -> Value {
    let mut m = u.to_public();
    m["network"] = json!(if crate::media::playback::home_network(&ctx.ip) { "home" } else { "away" });
    m
}

/// Checks the current password for sensitive changes (brute-force protected like the sign-in).
fn require_password(app: &AppState, ctx: &Ctx, body: &JsonBody) -> Result<(), ApiError> {
    let u = ctx.user()?;
    throttle(app, ctx, &u.username)?;
    if !auth::verify_password(body.str("currentPassword").unwrap_or(""), &u.password_hash) {
        app.guard.failure(&ctx.ip, &u.username);
        return Err(ApiError::new(StatusCode::FORBIDDEN, "error.wrongCurrentPassword"));
    }
    Ok(())
}

pub async fn change_password(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    require_password(&app, &ctx, &body)?;
    let mut u = ctx.user()?.clone();
    u.password_hash = auth::hash_password(&valid_password(&app, body.str("password").unwrap_or(""), &u.username)?);
    u.password_changed_at = now_ms();
    app.db.update_user(&u);
    let ended = app.db.remove_sessions_of(&u.id, ctx.token.as_deref());
    tracing::info!("Password of '{}' changed, {} other sessions ended", u.username, ended);
    ok(json!({"endedSessions": ended}))
}

// ====================================================================== switching users with a PIN

/// Sets (`pin`: 4 to 8 digits) or removes (empty) the PIN of the own account. Admins sign in with
/// their password only, so a PIN cannot open the administration.
pub async fn set_pin(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    require_password(&app, &ctx, &body)?;
    let mut u = ctx.user()?.clone();
    let pin = body.string("pin").trim().to_string();
    if pin.is_empty() {
        u.pin_hash = None;
    } else {
        if u.admin {
            return Err(ApiError::bad_request("error.pinAdmin"));
        }
        if !(4..=8).contains(&pin.len()) || !pin.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ApiError::bad_request("error.pinFormat"));
        }
        u.pin_hash = Some(auth::hash_password(&pin));
    }
    app.db.update_user(&u);
    ok(u.to_public())
}

/// The accounts one can switch to from a signed-in device: those with a PIN (never admins).
pub async fn switch_targets(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    let me = ctx.user()?;
    let list: Vec<Value> = app
        .db
        .users()
        .into_iter()
        .filter(|u| u.pin_hash.is_some() && !u.admin && u.id != me.id)
        .map(|u| json!({"id": u.id, "username": u.username, "avatar": u.avatar_version}))
        .collect();
    ok(json!(list))
}

/// Switches this device to another account with its PIN: the current session ends, a new one
/// starts. Only from a signed-in device, and brute-force protected like the sign-in. A wrong PIN
/// is 403, not 401: the device stays signed in as before.
pub async fn switch_user(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    ctx.user()?;
    let target = app.db.user(body.str("userId").unwrap_or("")).filter(|u| !u.admin);
    let Some(u) = target else { return Err(ApiError::new(StatusCode::FORBIDDEN, "error.wrongPin")) };
    throttle(&app, &ctx, &u.username)?;
    let pin = body.string("pin");
    let hash = u.pin_hash.clone();
    let ok_pin = match hash {
        Some(h) => tokio::task::spawn_blocking(move || auth::verify_password(&pin, &h)).await.unwrap_or(false),
        None => false,
    };
    if !ok_pin {
        app.guard.failure(&ctx.ip, &u.username);
        app.db.add_login_event(&u.id, &event(&ctx, false, "wrong-pin"));
        tracing::warn!("Wrong PIN for '{}' from {}", u.username, ctx.ip);
        return Err(ApiError::new(StatusCode::FORBIDDEN, "error.wrongPin"));
    }
    app.guard.success(&ctx.ip, &u.username);
    if let Some(t) = &ctx.token {
        app.db.remove_session(t);
    }
    start_session(&app, &ctx, &u, "pin")
}

pub async fn sessions(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    let u = ctx.user()?;
    let current = ctx.token.as_deref().map(session_id).unwrap_or_default();
    let list: Vec<Value> = app
        .db
        .sessions_of(&u.id)
        .into_iter()
        .map(|s| json!({"id": s.id, "current": s.id == current, "createdAt": s.created_at, "lastSeen": s.last_seen, "ip": s.ip, "userAgent": s.user_agent}))
        .collect();
    ok(json!(list))
}

pub async fn end_session(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(sid): Path<String>) -> ApiResult {
    if !app.db.remove_session_by_id(&ctx.user()?.id, &sid) {
        return Err(ApiError::not_found());
    }
    no_content()
}

pub async fn logout_others(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    let ended = app.db.remove_sessions_of(&ctx.user()?.id, ctx.token.as_deref());
    ok(json!({"ended": ended}))
}

pub async fn logins(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    const KNOWN: [&str; 7] = ["password", "password+totp", "password+recovery", "wrong-password", "wrong-code", "pin", "wrong-pin"];
    let list: Vec<Value> = app
        .db
        .login_events(&ctx.user()?.id)
        .into_iter()
        .map(|e| {
            let detail = if KNOWN.contains(&e.detail.as_str()) { ctx.t(&Text::new(&format!("login.{}", e.detail))) } else { e.detail };
            json!({"time": e.time, "ip": e.ip, "userAgent": e.user_agent, "success": e.success, "detail": detail})
        })
        .collect();
    ok(json!(list))
}

pub async fn two_factor_setup(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    require_password(&app, &ctx, &body)?;
    let mut u = ctx.user()?.clone();
    if u.totp_enabled {
        return Err(ApiError::bad_request("error.twoFactorActive"));
    }
    let secret = auth::totp_secret();
    u.totp_secret = Some(secret.clone());
    u.totp_last_step = 0;
    app.db.update_user(&u);
    ok(json!({"secret": secret, "uri": auth::totp_uri(&app.config.server.name, &u.username, &secret)}))
}

pub async fn two_factor_enable(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    let mut u = ctx.user()?.clone();
    if u.totp_enabled {
        return Err(ApiError::bad_request("error.twoFactorActive"));
    }
    let secret = u.totp_secret.clone().ok_or_else(|| ApiError::bad_request("error.twoFactorRestart"))?;
    let step =
        auth::totp_verify(&secret, body.str("code").unwrap_or(""), 0).ok_or_else(|| ApiError::bad_request("error.wrongCodeCheckClock"))?;
    let (plain, hashes) = auth::recovery_codes();
    u.recovery_codes = hashes;
    u.totp_last_step = step;
    u.totp_enabled = true;
    app.db.update_user(&u);
    let ended = app.db.remove_sessions_of(&u.id, ctx.token.as_deref());
    tracing::info!("Two-factor sign-in enabled for '{}', {} other sessions ended", u.username, ended);
    ok(json!({"recoveryCodes": plain}))
}

pub async fn two_factor_disable(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    require_password(&app, &ctx, &body)?;
    let mut u = ctx.user()?.clone();
    u.totp_enabled = false;
    u.totp_secret = None;
    u.recovery_codes.clear();
    app.db.update_user(&u);
    tracing::info!("Two-factor sign-in disabled for '{}'", u.username);
    ok(u.to_public())
}

pub async fn new_recovery_codes(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    require_password(&app, &ctx, &body)?;
    let mut u = ctx.user()?.clone();
    if !u.totp_enabled {
        return Err(ApiError::bad_request("error.twoFactorInactive"));
    }
    let (plain, hashes) = auth::recovery_codes();
    u.recovery_codes = hashes;
    app.db.update_user(&u);
    ok(json!({"recoveryCodes": plain}))
}

pub async fn preferences(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    let mut u = ctx.user()?.clone();
    let field = |key: &str, current: &str| body.str(key).unwrap_or(current).to_string();
    let audio = field("audioLanguage", &u.audio_language);
    let mode = field("subtitleMode", &u.subtitle_mode);
    let subs = field("subtitleLanguage", &u.subtitle_language);
    let ui = field("uiLanguage", &u.ui_language);
    if !ui.is_empty() && !i18n::supported(&ui) {
        return Err(ApiError::bad_request("error.unknownLanguage").arg(ui));
    }
    for l in [&audio, &subs] {
        if !l.is_empty() && !MEDIA_LANGUAGES.contains(&l.as_str()) {
            return Err(ApiError::bad_request("error.unknownLanguage").arg(l.clone()));
        }
    }
    if !SUBTITLE_MODES.contains(&mode.as_str()) {
        return Err(ApiError::bad_request("error.unknownSubtitleMode").arg(mode));
    }
    if let Some(v) = body.get("unpinned") {
        u.unpinned = nav_entries(v)?;
    }
    // {"subtitleStyle": {"size": "l", …}}: only known settings and values are kept.
    if let Some(style) = body.get("subtitleStyle") {
        let style = style.as_object().ok_or_else(|| ApiError::bad_request("error.subtitleStyle"))?;
        let current = u.subtitle_style_json();
        let mut out = url::form_urlencoded::Serializer::new(String::new());
        for (key, values) in crate::db::SUBTITLE_STYLE {
            let v = style.get(key).or_else(|| current.get(key)).and_then(Value::as_str).unwrap_or(values[0]);
            if !values.contains(&v) {
                return Err(ApiError::bad_request("error.subtitleStyle"));
            }
            out.append_pair(key, v);
        }
        u.subtitle_style = out.finish();
    }
    for (key, slot) in [("qualityHome", &mut u.quality_home), ("qualityAway", &mut u.quality_away)] {
        if let Some(q) = body.str(key) {
            if !crate::media::playback::known_quality(q) {
                return Err(ApiError::bad_request("error.unknownQuality").arg(q.to_string()));
            }
            *slot = q.to_string();
        }
    }
    (u.audio_language, u.subtitle_mode, u.subtitle_language, u.ui_language) = (audio, mode, subs, ui);
    app.db.update_user(&u);
    ok(u.to_public())
}

/// Sidebar entries such as `history` or `library:<id>`.
fn nav_entries(v: &Value) -> Result<Vec<String>, ApiError> {
    let invalid = || ApiError::bad_request("error.unpinnedValue");
    let list = v.as_array().filter(|l| l.len() <= 200).ok_or_else(invalid)?;
    let mut out: Vec<String> = Vec::new();
    for e in list {
        let s = e.as_str().ok_or_else(invalid)?;
        let (name, id) = s.split_once(':').map_or((s, None), |(n, i)| (n, Some(i)));
        let name_ok = (1..=20).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_lowercase());
        let id_ok =
            id.is_none_or(|i| (1..=64).contains(&i.len()) && i.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'));
        if !name_ok || !id_ok {
            return Err(invalid());
        }
        if !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    }
    Ok(out)
}

/// 2 to 32 letters, digits, spaces, dots, dashes and underscores.
pub fn valid_username(s: &str) -> Result<String, ApiError> {
    let u = s.trim();
    let n = u.chars().count();
    if (2..=32).contains(&n) && u.chars().all(|c| c.is_alphanumeric() || "._ -".contains(c)) {
        Ok(u.to_string())
    } else {
        Err(ApiError::bad_request("error.usernameRules"))
    }
}

pub fn valid_password(app: &AppState, s: &str, username: &str) -> Result<String, ApiError> {
    let min = app.config.security.min_password_length;
    let n = s.chars().count();
    if n < min {
        return Err(ApiError::bad_request("error.passwordTooShort").arg(min));
    }
    if n > 256 {
        return Err(ApiError::bad_request("error.passwordTooLong"));
    }
    if s.to_lowercase() == username.to_lowercase() {
        return Err(ApiError::bad_request("error.passwordIsUsername"));
    }
    let mut distinct: Vec<char> = s.chars().collect();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() < 4 {
        return Err(ApiError::bad_request("error.passwordTooSimple"));
    }
    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        assert!(valid_username("Jörg M.").is_ok());
        assert!(valid_username("a").is_err());
        assert!(valid_username("bad<script>").is_err());
    }

    #[test]
    fn sidebar_entries() {
        assert_eq!(nav_entries(&json!(["history", "library:ab12", "history"])).unwrap(), vec!["history", "library:ab12"]);
        assert!(nav_entries(&json!(["Bad"])).is_err());
        assert!(nav_entries(&json!("history")).is_err());
    }
}
