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

//! Compatibility layer for native Jellyfin apps (Android TV and Fire TV, Findroid, Swiftfin and
//! the other apps that talk to the API directly; apps that show the Jellyfin web interface are
//! not supported). It is served under `/jellyfin`, so the apps get
//! `http://server:port/jellyfin` as server address, and is off unless `[jellyfin] enabled`.
//!
//! Nothing here keeps state of its own beyond a running playback and a pending Quick Connect:
//! sign-ins are FreeX sessions, watch state is FreeX watch state, favorites are the watchlist,
//! and playback goes through the FreeX playback decision and conversion. See docs/jellyfin.md.
//!
//! Ids are GUIDs as the apps expect them: an 8-digit type prefix and the FreeX id, padded
//! ([`guid`], [`parse_guid`]). Paths are matched without regard to case, in the form of the
//! current API (`/Items/{id}`) and the older per-user form (`/Users/{user}/Items/{id}`).

mod browse;
mod dto;
mod login;
mod play;

pub use login::authorize_quick_connect;

use crate::AppState;
use crate::db::User;
use crate::http::{ApiError, Ctx};
use axum::Extension;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The address prefix of the layer.
pub const BASE: &str = "/jellyfin";
/// The API version the apps are told: apps built for 12.x refuse older servers, those for 10.11
/// accept newer ones.
pub const API_VERSION: &str = "12.4.0";
/// The apps check this name to tell a media server from another service on the same address.
pub const PRODUCT_NAME: &str = "Jellyfin Server";

/// Start of the user agent FreeX records for requests of the apps.
const UA_MARKER: &str = "Jellyfin app; ";

/// Type prefixes of the GUIDs.
const T_ITEM: &str = "f4ee0001";
const T_LIBRARY: &str = "f4ee0002";
const T_USER: &str = "f4ee0003";
const T_GENRE: &str = "f4ee0004";

/// What the layer keeps between requests.
pub struct Jellyfin {
    /// Signs image tags; kept in `<data>/jellyfin.key`, so tags stay valid across restarts.
    key: Vec<u8>,
    /// Stable id of this server for the apps, derived from the key.
    pub server_id: String,
    quick: Mutex<HashMap<String, login::QuickConnect>>,
    plays: Mutex<HashMap<String, play::Play>>,
}

impl Jellyfin {
    /// Reads the key from the data directory, creating it on the first start.
    pub fn load(data_dir: &Path) -> Jellyfin {
        let path = data_dir.join("jellyfin.key");
        let key = match std::fs::read_to_string(&path).ok().and_then(|s| hex::decode(s.trim()).ok()).filter(|k| k.len() == 32) {
            Some(k) => k,
            None => {
                let mut k = vec![0u8; 32];
                rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut k);
                if let Err(e) = write_private(&path, &hex::encode(&k)) {
                    tracing::warn!("Cannot write {}: {e}; image links of Jellyfin apps change with every start", path.display());
                }
                k
            }
        };
        let server_id = crate::auth::sha256(&format!("server\0{}", hex::encode(&key)))[..32].to_string();
        Jellyfin { key, server_id, quick: Mutex::new(HashMap::new()), plays: Mutex::new(HashMap::new()) }
    }

    /// A short signature of `data`, used as image tag.
    fn sign(&self, data: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("any key length");
        mac.update(data.as_bytes());
        hex::encode(&mac.finalize().into_bytes()[..12])
    }
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

// ====================================================================== ids

/// The GUID of a FreeX id (16 or 24 hex digits) with a type prefix.
fn guid(tag: &str, id: &str) -> String {
    format!("{tag}{id:0>24}")
}

pub(super) fn item_guid(id: &str) -> String {
    guid(T_ITEM, id)
}

fn library_guid(id: &str) -> String {
    guid(T_LIBRARY, id)
}

fn user_guid(id: &str) -> String {
    guid(T_USER, id)
}

fn genre_guid(name: &str) -> String {
    guid(T_GENRE, &crate::auth::sha256(&name.trim().to_lowercase())[..24])
}

/// What a GUID stands for.
#[derive(Debug, PartialEq)]
enum Id {
    Item(String),
    Library(String),
    User(String),
    Genre(String),
}

/// Reads a GUID with or without dashes, in any case.
fn parse_guid(s: &str) -> Option<Id> {
    let s: String = s.chars().filter(|c| *c != '-').collect::<String>().to_ascii_lowercase();
    if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let (tag, rest) = s.split_at(8);
    match tag {
        // Item ids have 16 digits, the rest of the 24 are the padding.
        T_ITEM => rest.strip_prefix("00000000").map(|id| Id::Item(id.to_string())),
        T_LIBRARY => Some(Id::Library(rest.to_string())),
        T_USER => Some(Id::User(rest.to_string())),
        T_GENRE => Some(Id::Genre(rest.to_string())),
        _ => None,
    }
}

// ====================================================================== request

/// The app that sends a request, from its `Authorization: MediaBrowser ...` header.
#[derive(Clone, Debug, Default)]
pub(super) struct Client {
    pub app: String,
    pub device: String,
    pub device_id: String,
    pub version: String,
    pub token: Option<String>,
}

impl Client {
    /// Short id of the device for the playback list (FreeX accepts only letters, digits, - and _).
    pub fn id(&self) -> String {
        format!("jf-{}", &crate::auth::sha256(&format!("{}\0{}", self.device_id, self.app))[..16])
    }

    /// Shown in the session list and the sign-in log of FreeX: "Findroid 0.15.2 (Pixel 8)".
    pub fn label(&self) -> String {
        let app = if self.app.is_empty() { "Jellyfin app" } else { &self.app };
        let mut s = app.to_string();
        if !self.version.is_empty() {
            s = format!("{s} {}", self.version);
        }
        if !self.device.is_empty() {
            s = format!("{s} ({})", self.device);
        }
        s.chars().take(200).collect()
    }
}

/// Parses `MediaBrowser Client="Android TV", Device="SHIELD", DeviceId="...", Version="0.18.0", Token="..."`.
fn parse_authorization(value: &str) -> Client {
    let mut c = Client::default();
    let v = value.trim();
    let Some((scheme, rest)) = v.split_once(' ') else { return c };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "mediabrowser" | "emby") {
        return c;
    }
    for part in rest.split(',') {
        let Some((k, val)) = part.trim().split_once('=') else { continue };
        let val = percent_decode(val.trim().trim_matches('"'));
        match k.trim().to_ascii_lowercase().as_str() {
            "client" => c.app = val,
            "device" => c.device = val,
            "deviceid" => c.device_id = val,
            "version" => c.version = val,
            "token" => c.token = Some(val).filter(|t| !t.is_empty()),
            _ => {}
        }
    }
    c
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(if b[i] == b'+' { b' ' } else { b[i] });
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One request to the layer.
pub(super) struct Req {
    pub method: Method,
    /// Path segments below `/jellyfin`, as sent and in lower case.
    pub segs: Vec<String>,
    pub low: Vec<String>,
    /// Query parameters by lower-case name; repeated names collect their values.
    pub query: HashMap<String, Vec<String>>,
    pub headers: HeaderMap,
    /// The JSON body, Null if there is none.
    pub body: Value,
    pub client: Client,
}

impl Req {
    /// A query parameter, or else a field of the JSON body, both without regard to case.
    pub fn param(&self, name: &str) -> Option<String> {
        let lower = name.to_ascii_lowercase();
        if let Some(v) = self.query.get(&lower).and_then(|v| v.first()) {
            return Some(v.clone());
        }
        self.field(name).and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        })
    }

    /// A field of the JSON body without regard to case.
    pub fn field(&self, name: &str) -> Option<&Value> {
        self.body.as_object()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v).filter(|v| !v.is_null())
    }

    /// A list parameter: comma separated, repeated, or a JSON array.
    pub fn list(&self, name: &str) -> Vec<String> {
        let lower = name.to_ascii_lowercase();
        let mut out: Vec<String> = Vec::new();
        if let Some(vals) = self.query.get(&lower) {
            for v in vals {
                out.extend(v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
            }
        } else if let Some(Value::Array(a)) = self.field(name) {
            out.extend(a.iter().filter_map(|v| v.as_str().map(str::to_string)));
        } else if let Some(s) = self.param(name) {
            out.extend(s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
        }
        out
    }

    /// A list parameter in lower case, for comparing names of types, filters and sort keys.
    pub fn list_lower(&self, name: &str) -> Vec<String> {
        self.list(name).into_iter().map(|s| s.to_ascii_lowercase()).collect()
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        self.param(name).and_then(|v| v.trim().parse::<i64>().ok().or_else(|| v.trim().parse::<f64>().ok().map(|f| f as i64)))
    }

    pub fn bool(&self, name: &str) -> Option<bool> {
        self.param(name).and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        })
    }
}

/// A request with its signed-in user.
pub(super) struct Cx {
    pub app: Arc<AppState>,
    /// The FreeX request context, with the user of the token.
    pub ctx: Ctx,
    pub req: Req,
}

/// An answer that ends a handler early; boxed, since responses are large.
pub(super) struct Fail(Box<Response>);

impl From<Response> for Fail {
    fn from(r: Response) -> Fail {
        Fail(Box::new(r))
    }
}

impl From<StatusCode> for Fail {
    fn from(s: StatusCode) -> Fail {
        Fail(Box::new(s.into_response()))
    }
}

/// Results of the handlers: an answer either way.
type Out = Result<Response, Fail>;

impl Cx {
    pub fn user(&self) -> Result<&User, Fail> {
        self.ctx.user.as_ref().ok_or(Fail::from(StatusCode::UNAUTHORIZED))
    }

    pub fn token(&self) -> &str {
        self.ctx.token.as_deref().unwrap_or("")
    }

    pub fn jf(&self) -> &Jellyfin {
        &self.app.jellyfin
    }
}

/// FreeX errors keep their status; the body is FreeX's error JSON, which the apps only log.
fn err(e: ApiError) -> Fail {
    Fail::from(e.into_response())
}

fn json_out(v: Value) -> Out {
    Ok(axum::Json(v).into_response())
}

fn empty(status: StatusCode) -> Out {
    Ok(status.into_response())
}

fn not_found() -> Fail {
    Fail::from(StatusCode::NOT_FOUND)
}

/// An empty list as the apps expect it.
fn query_result(items: Vec<Value>, total: usize, start: usize) -> Value {
    json!({"Items": items, "TotalRecordCount": total, "StartIndex": start})
}

// ====================================================================== entry

/// Everything under `/jellyfin`.
pub async fn handle(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !app.config.jellyfin.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let started = std::time::Instant::now();
    let path = uri.path().strip_prefix(BASE).unwrap_or("");
    let mut segs: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(percent_decode).collect();
    // Some apps put the old prefixes in front.
    if segs.first().is_some_and(|s| s.eq_ignore_ascii_case("emby") || s.eq_ignore_ascii_case("mediabrowser")) {
        segs.remove(0);
    }
    let segs = modern_path(segs);
    let low = segs.iter().map(|s| s.to_ascii_lowercase()).collect();

    let mut query: HashMap<String, Vec<String>> = HashMap::new();
    for (k, v) in url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes()) {
        query.entry(k.to_ascii_lowercase()).or_default().push(v.into_owned());
    }
    let body = if body.is_empty() { Value::Null } else { serde_json::from_slice(&body).unwrap_or(Value::Null) };

    let mut client = ["authorization", "x-emby-authorization"]
        .iter()
        .filter_map(|h| headers.get(*h).and_then(|v| v.to_str().ok()))
        .map(parse_authorization)
        .find(|c| !c.app.is_empty() || c.token.is_some())
        .unwrap_or_default();
    let token = client
        .token
        .clone()
        .or_else(|| {
            ["x-emby-token", "x-mediabrowser-token"].iter().find_map(|h| headers.get(*h).and_then(|v| v.to_str().ok()).map(str::to_string))
        })
        .or_else(|| query.get("api_key").or_else(|| query.get("apikey")).and_then(|v| v.first().cloned()))
        .filter(|t| !t.is_empty() && t.len() < 200);
    client.token = token.clone();

    let mut ctx = ctx;
    // FreeX Web shows the part after the marker as device name (sessions, sign-in log).
    ctx.user_agent = format!("{UA_MARKER}{}", client.label());
    if let Some(t) = &token {
        let max_idle = i64::from(app.config.security.session_days) * 86_400_000;
        if let Some(u) = app.db.user_for_token(t, max_idle, &ctx.ip) {
            ctx.user = Some(u);
            ctx.token = Some(t.clone());
        }
    }
    let cx = Cx { app, ctx, req: Req { method, segs, low, query, headers, body, client } };
    let mut r = match route(&cx).await {
        Ok(r) => r,
        Err(Fail(r)) => *r,
    };
    // Every request with its answer, for finding out what an app does (without the query, which
    // can hold the access token).
    let status = r.status();
    // The time until the answer starts; a file is sent after that, as fast as the network takes it.
    let ms = started.elapsed().as_millis();
    let range = cx.req.headers.get(header::RANGE).and_then(|v| v.to_str().ok()).map(|r| format!(" {r}")).unwrap_or_default();
    let line = format!("{} /{}{range} → {} in {ms} ms ({})", cx.req.method, cx.req.segs.join("/"), status.as_u16(), cx.req.client.label());
    if status.is_server_error() {
        tracing::warn!("Jellyfin layer: {line}");
    } else {
        tracing::debug!("Jellyfin layer: {line}");
    }
    // The answers are for apps, not for pages in a browser.
    r.headers_mut().remove("content-security-policy");
    r
}

/// Turns the older per-user paths into the current ones:
/// `/Users/{u}/Items/{id}` → `/Items/{id}`, `/Users/{u}/Views` → `/UserViews`, and so on.
fn modern_path(segs: Vec<String>) -> Vec<String> {
    let low: Vec<String> = segs.iter().map(|s| s.to_ascii_lowercase()).collect();
    if low.len() < 3 || low[0] != "users" || parse_guid(&segs[1]).is_none() {
        return segs;
    }
    let rest = &segs[2..];
    let lrest: Vec<&str> = low[2..].iter().map(String::as_str).collect();
    let v = |parts: &[&str], tail: &[String]| -> Vec<String> { parts.iter().map(|s| s.to_string()).chain(tail.iter().cloned()).collect() };
    match lrest.as_slice() {
        ["views"] => v(&["UserViews"], &[]),
        ["items", "resume"] => v(&["UserItems", "Resume"], &[]),
        ["items", "latest"] => v(&["Items", "Latest"], &[]),
        ["items", _, "userdata"] => v(&["UserItems"], &rest[1..]),
        ["items", ..] => v(&[], rest),
        ["favoriteitems", _] => v(&["UserFavoriteItems"], &rest[1..]),
        ["playeditems", _] => v(&["UserPlayedItems"], &rest[1..]),
        ["playingitems", ..] => v(&["PlayingItems"], &rest[1..]),
        ["suggestions"] => v(&["Items", "Suggestions"], &[]),
        _ => segs,
    }
}

async fn route(cx: &Cx) -> Out {
    let m = cx.req.method.as_str();
    let p: Vec<&str> = cx.req.low.iter().map(String::as_str).collect();
    let id = |i: usize| cx.req.segs.get(i).map(String::as_str).unwrap_or("");
    let read = m == "GET" || m == "HEAD";
    match (m, p.as_slice()) {
        // ---------------------------------------------------------- server, without sign-in
        (_, ["system", "info", "public"]) => json_out(public_info(cx)),
        (_, ["system", "ping"]) => Ok(PRODUCT_NAME.into_response()),
        ("GET", ["system", "info"]) => {
            cx.user()?;
            json_out(system_info(cx))
        }
        ("GET", ["system", "endpoint"]) => {
            let local = crate::media::playback::home_network(&cx.ctx.ip);
            json_out(json!({"IsLocal": local, "IsInNetwork": local}))
        }
        ("GET", ["branding", "configuration"]) => json_out(json!({"LoginDisclaimer": "", "CustomCss": "", "SplashscreenEnabled": false})),
        ("GET", ["branding", "css" | "css.css"]) => Ok(([(header::CONTENT_TYPE, "text/css")], "").into_response()),
        ("GET", ["users", "public"]) => json_out(json!([])),
        ("GET", ["quickconnect", "enabled"]) => json_out(json!(true)),
        ("POST", ["users", "authenticatebyname"]) => login::by_name(cx).await,
        ("POST", ["quickconnect", "initiate"]) | ("GET", ["quickconnect", "initiate"]) => login::initiate(cx),
        ("GET", ["quickconnect", "connect"]) => login::state(cx),
        ("POST", ["users", "authenticatewithquickconnect"]) => login::with_quick_connect(cx),
        ("POST", ["quickconnect", "authorize"]) => login::authorize(cx),

        // ---------------------------------------------------------- images (signed tag or sign-in)
        (_, ["items", _, "images", kind, ..]) if read => browse::image(cx, id(1), kind).await,

        // ---------------------------------------------------------- playback files (own access)
        (_, ["videos", _, "hls", _, _]) if read => play::hls_file(cx).await,
        // The file itself: a sign-in, or the PlaySessionId of the user's playback info.
        (
            _,
            [
                "videos",
                _,
                "stream" | "stream.mkv" | "stream.mp4" | "stream.m4v" | "stream.webm" | "stream.avi" | "stream.ts" | "stream.mov",
            ],
        ) if read => play::stream(cx, id(1)).await,

        // ---------------------------------------------------------- everything else needs a sign-in
        _ if cx.ctx.user.is_none() => Err(StatusCode::UNAUTHORIZED.into()),

        ("GET", ["users", "me"]) => json_out(login::user_dto(cx, cx.user()?)),
        ("GET", ["users"]) => json_out(json!([login::user_dto(cx, cx.user()?)])),
        ("GET", ["users", u]) => match parse_guid(u) {
            Some(Id::User(uid)) if uid == cx.user()?.id => json_out(login::user_dto(cx, cx.user()?)),
            _ => Err(not_found()),
        },
        ("POST", ["sessions", "logout"]) => {
            cx.app.db.remove_session(cx.token());
            empty(StatusCode::NO_CONTENT)
        }
        ("GET", ["sessions"]) => json_out(json!([])),
        ("POST", ["sessions", "capabilities", ..]) | ("POST", ["devices", "options"]) | ("POST", ["displaypreferences", _]) => {
            empty(StatusCode::NO_CONTENT)
        }
        ("POST", ["clientlog", "document"]) => json_out(json!({"FileName": ""})),
        ("GET", ["displaypreferences", pid]) => json_out(display_preferences(pid, &cx.req.client.app)),
        ("GET", ["playback", "bitratetest"]) => bitrate_test(cx),

        // ---------------------------------------------------------- browsing
        ("GET", ["userviews"]) | ("GET", ["library", "mediafolders"]) => browse::views(cx),
        ("GET", ["items"]) => browse::items(cx),
        ("GET", ["items", "latest"]) => browse::latest(cx),
        ("GET", ["items", "resume"]) | ("GET", ["useritems", "resume"]) => browse::resume(cx),
        ("GET", ["items", "suggestions"]) => json_out(query_result(vec![], 0, 0)),
        ("GET", ["items", "filters"]) => browse::filters_legacy(cx),
        ("GET", ["items", "filters2"]) => browse::filters(cx),
        ("GET", ["items", _]) => browse::item(cx, id(1)),
        ("GET", ["items", _, "similar"]) => browse::similar(cx, id(1)),
        ("GET", ["items", _, "specialfeatures"]) => browse::extras(cx, id(1), false),
        ("GET", ["items", _, "localtrailers"]) => browse::extras(cx, id(1), true),
        ("GET", ["items", _, "intros"]) | ("GET", ["videos", _, "additionalparts"]) => json_out(query_result(vec![], 0, 0)),
        ("GET", ["items", _, "thememedia" | "themesongs" | "themevideos"]) => json_out(browse::no_theme_media()),
        ("GET", ["shows", "nextup"]) => browse::next_up(cx),
        ("GET", ["shows", "upcoming"]) => json_out(query_result(vec![], 0, 0)),
        ("GET", ["shows", _, "seasons"]) => browse::seasons(cx, id(1)),
        ("GET", ["shows", _, "episodes"]) => browse::episodes(cx, id(1)),
        ("GET", ["genres"]) => browse::genres(cx),
        ("GET", ["persons" | "studios"]) => json_out(query_result(vec![], 0, 0)),
        ("GET", ["mediasegments", _]) => browse::segments(cx, id(1)),

        // ---------------------------------------------------------- watch state and favorites
        ("POST", ["userplayeditems", _]) => play::set_played(cx, id(1), true).await,
        ("DELETE", ["userplayeditems", _]) => play::set_played(cx, id(1), false).await,
        ("POST", ["userfavoriteitems", _]) => play::set_favorite(cx, id(1), true),
        ("DELETE", ["userfavoriteitems", _]) => play::set_favorite(cx, id(1), false),
        ("GET", ["useritems", _, "userdata"]) => play::user_data(cx, id(1)),
        ("POST", ["useritems", _, "userdata"]) => play::update_user_data(cx, id(1)).await,

        // ---------------------------------------------------------- playback
        ("GET" | "POST", ["items", _, "playbackinfo"]) => play::playback_info(cx, id(1)),
        (_, ["videos", _, "master.m3u8" | "main.m3u8"]) if read => play::hls_playlist(cx, id(1)).await,
        ("GET", ["videos", _, _, "subtitles", index, .., file]) if file.starts_with("stream.") => {
            play::subtitle(cx, id(1), id(2), index, file).await
        }
        ("POST", ["sessions", "playing"]) => play::started(cx).await,
        ("POST", ["sessions", "playing", "progress"]) => play::progress(cx, false).await,
        ("POST", ["sessions", "playing", "stopped"]) => play::progress(cx, true).await,
        ("POST", ["sessions", "playing", "ping"]) => empty(StatusCode::NO_CONTENT),
        ("DELETE", ["videos", "activeencodings"]) => play::stop_encoding(cx),
        ("POST", ["playingitems", _]) => play::started(cx).await,
        ("POST", ["playingitems", _, "progress"]) => play::progress(cx, false).await,
        ("DELETE", ["playingitems", _]) => play::progress(cx, true).await,

        _ => {
            tracing::debug!("Jellyfin layer: no handler for {m} /{}", cx.req.segs.join("/"));
            Err(not_found())
        }
    }
}

// ====================================================================== server

fn public_info(cx: &Cx) -> Value {
    json!({
        "LocalAddress": local_address(cx),
        "ServerName": cx.app.config.server.name,
        "Version": API_VERSION,
        "ProductName": PRODUCT_NAME,
        "OperatingSystem": "",
        "Id": cx.jf().server_id,
        "StartupWizardCompleted": true,
    })
}

fn system_info(cx: &Cx) -> Value {
    let mut v = public_info(cx);
    let extra = json!({
        "OperatingSystemDisplayName": std::env::consts::OS,
        "HasPendingRestart": false,
        "IsShuttingDown": false,
        "SupportsLibraryMonitor": false,
        "WebSocketPortNumber": cx.app.config.server.port,
        "CompletedInstallations": [],
        "CanSelfRestart": false,
        "CanLaunchWebBrowser": false,
        "ProgramDataPath": "",
        "WebPath": "",
        "ItemsByNamePath": "",
        "CachePath": "",
        "LogPath": "",
        "InternalMetadataPath": "",
        "TranscodingTempPath": "",
        "CastReceiverApplications": [],
        "HasUpdateAvailable": false,
        "EncoderLocation": "System",
        "SystemArchitecture": std::env::consts::ARCH,
    });
    if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
        o.extend(e);
    }
    v
}

/// The address the request came in on, with the prefix of the layer.
fn local_address(cx: &Cx) -> String {
    let h = &cx.req.headers;
    let trust = cx.app.config.server.trust_proxy;
    let host = if trust { h.get("x-forwarded-host") } else { None }
        .or_else(|| h.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    let scheme = if cx.ctx.secure { "https" } else { "http" };
    format!("{scheme}://{host}{BASE}")
}

fn display_preferences(id: &str, client: &str) -> Value {
    json!({
        "Id": id,
        "SortBy": "SortName",
        "SortOrder": "Ascending",
        "RememberIndexing": false,
        "PrimaryImageHeight": 250,
        "PrimaryImageWidth": 250,
        "CustomPrefs": {},
        "ScrollDirection": "Horizontal",
        "ShowBackdrop": true,
        "RememberSorting": false,
        "ShowSidebar": false,
        "Client": client,
    })
}

/// Random bytes for the bandwidth measurement of some apps (at most 10 MB).
fn bitrate_test(cx: &Cx) -> Out {
    let size = cx.req.int("size").unwrap_or(102_400).clamp(1, 10_000_000) as usize;
    let mut data = vec![0u8; size];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut data);
    let mut r = Response::new(Body::from(data));
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_round_trip() {
        let item = "0123456789abcdef";
        let g = item_guid(item);
        assert_eq!(g.len(), 32);
        assert_eq!(parse_guid(&g), Some(Id::Item(item.into())));
        let dashed = format!("{}-{}-{}-{}-{}", &g[..8], &g[8..12], &g[12..16], &g[16..20], &g[20..]).to_uppercase();
        assert_eq!(parse_guid(&dashed), Some(Id::Item(item.into())));
        let user = "0123456789abcdef01234567";
        assert_eq!(parse_guid(&user_guid(user)), Some(Id::User(user.into())));
        assert_eq!(parse_guid(&library_guid(user)), Some(Id::Library(user.into())));
        assert_eq!(parse_guid("not a guid"), None);
        assert_eq!(parse_guid("00000000000000000000000000000000"), None);
    }

    #[test]
    fn authorization_header() {
        let c = parse_authorization(
            r#"MediaBrowser Client="Jellyfin%20Android%20TV", Device="SHIELD", DeviceId="abc=", Version="0.18.0", Token="t0k"#,
        );
        assert_eq!(c.app, "Jellyfin Android TV");
        assert_eq!(c.device, "SHIELD");
        assert_eq!(c.device_id, "abc=");
        assert_eq!(c.token.as_deref(), Some("t0k"));
        assert!(c.id().starts_with("jf-") && c.id().len() == 19);
        assert_eq!(c.label(), "Jellyfin Android TV 0.18.0 (SHIELD)");
        assert_eq!(parse_authorization("Bearer abc").token, None);
    }

    #[test]
    fn older_paths() {
        let u = user_guid("0123456789abcdef01234567");
        let p = |s: &str| modern_path(s.split('/').map(str::to_string).collect()).join("/");
        assert_eq!(p(&format!("Users/{u}/Views")), "UserViews");
        assert_eq!(p(&format!("Users/{u}/Items/Resume")), "UserItems/Resume");
        assert_eq!(p(&format!("Users/{u}/Items/abc")), "Items/abc");
        assert_eq!(p(&format!("Users/{u}/Items/abc/UserData")), "UserItems/abc/UserData");
        assert_eq!(p(&format!("Users/{u}/PlayedItems/abc")), "UserPlayedItems/abc");
        assert_eq!(p("Users/Public"), "Users/Public");
    }
}
