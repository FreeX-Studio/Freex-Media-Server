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

//! A DLNA/UPnP media server (DMS) for smart TVs, consoles, VLC, Kodi and other players in the home
//! network, off unless `[dlna] enabled`.
//!
//! It runs next to the main server: SSDP on UDP port 1900 so players find it ([`ssdp`]), and a
//! plain HTTP port of its own (`[dlna] port`, TVs cannot use HTTPS) for the description, the
//! ContentDirectory and ConnectionManager services ([`xml`], [`content`]) and the files. Both
//! answer only addresses of the home network.
//!
//! DLNA knows no sign-in, so everything is seen as the FreeX user `[dlna] user`: libraries, age
//! limit, resume positions and watch state are theirs. Files are sent as they are (players
//! decide what they can play). The position of a playback is estimated from the bytes sent; it
//! keeps the activity view up to date and becomes the resume position when the player stops.

mod content;
mod ssdp;
mod xml;

use crate::AppState;
use crate::api::media::{IMAGE_CACHE_SECONDS, image_path, may_see};
use crate::auth::now_ms;
use crate::db::User;
use crate::http::{Ctx, JsonBody};
use crate::media::Item;
use crate::media::playback::home_network;
use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Extension, Router};
use content::Tree;
use futures_core::Stream;
use serde_json::json;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant};

pub use ssdp::Info;

/// How often a running stream updates the activity view.
const REPORT_EVERY: Duration = Duration::from_secs(10);
/// What the player has buffered ahead of what it shows, roughly.
const BUFFER_MS: i64 = 10_000;
/// A request counts as watching once it sent this much (players also read a few MB at the end of
/// a file for its index, which must not count as having watched to the end).
const WATCHING_BYTES: u64 = 32 * 1_048_576;

/// Starts the media server when it is switched on; the result is needed for [`goodbye`].
pub fn start(app: &Arc<AppState>) -> Option<Arc<Info>> {
    let c = &app.config.dlna;
    if !c.enabled {
        return None;
    }
    if c.user.trim().is_empty() || app.db.user_by_name(c.user.trim()).is_none() {
        tracing::warn!("DLNA is on, but dlna.user '{}' is not a FreeX account; DLNA stays off", c.user);
        return None;
    }
    let addr = SocketAddr::from(([0, 0, 0, 0], c.port));
    let listener = match std::net::TcpListener::bind(addr).and_then(|l| l.set_nonblocking(true).map(|()| l)) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("DLNA cannot listen on port {}: {e}", c.port);
            return None;
        }
    };
    let info = Arc::new(Info {
        udn: udn(app),
        port: c.port,
        server: format!("{}/1.0 UPnP/1.0 FreeX/{}", std::env::consts::OS, crate::api::VERSION),
        boot_id: (now_ms() / 1000) as u32,
    });
    let service = router(app.clone(), info.clone()).into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move {
        match axum_server::from_tcp(listener) {
            Ok(s) => {
                if let Err(e) = s.serve(service).await {
                    tracing::warn!("DLNA HTTP server ended: {e}");
                }
            }
            Err(e) => tracing::warn!("DLNA HTTP server: {e}"),
        }
    });
    let ssdp_info = info.clone();
    tokio::spawn(async move {
        if let Err(e) = ssdp::run(ssdp_info).await {
            tracing::warn!("DLNA discovery (UDP port {}) is not available: {e}. Players can still add the server by address.", ssdp::PORT);
        }
    });
    tracing::info!("DLNA media server '{}' on port {} for the libraries of '{}'", friendly_name(app), c.port, c.user.trim());
    Some(info)
}

/// Tells the players that the server goes away.
pub fn goodbye(info: &Info) {
    ssdp::goodbye(info);
}

fn friendly_name(app: &AppState) -> String {
    let n = app.config.dlna.name.trim();
    if n.is_empty() { app.config.server.name.clone() } else { n.to_string() }
}

/// A stable device id: derived from the server's own key, so players keep their settings for it.
fn udn(app: &AppState) -> String {
    let h = crate::auth::sha256(&format!("dlna\0{}", app.jellyfin.server_id));
    format!("uuid:{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn router(app: Arc<AppState>, info: Arc<Info>) -> Router {
    Router::new()
        .route("/dlna/description.xml", get(description))
        .route("/dlna/cd.xml", get(|| async { xml_response(xml::content_directory()) }))
        .route("/dlna/cm.xml", get(|| async { xml_response(xml::connection_manager()) }))
        .route("/dlna/icon.png", get(icon))
        .route("/dlna/control/cd", post(control_cd))
        .route("/dlna/control/cm", post(control_cm))
        .route("/dlna/event/{service}", any(event))
        .route("/dlna/media/{id}", get(media))
        .route("/dlna/image/{id}", get(image))
        .route("/dlna/subtitle/{id}/{file}", get(subtitle))
        .layer(axum::middleware::from_fn(home_only))
        .layer(Extension(info))
        .with_state(app)
}

/// Only the home network; and the header players expect on every answer.
async fn home_only(ConnectInfo(addr): ConnectInfo<SocketAddr>, req: Request, next: Next) -> Response {
    if !home_network(&addr.ip().to_string()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut r = next.run(req).await;
    r.headers_mut().insert("ext", HeaderValue::from_static(""));
    r
}

fn xml_response(body: String) -> Response {
    ([(header::CONTENT_TYPE, "text/xml; charset=\"utf-8\"")], body).into_response()
}

/// `http://host:port` as the player reached this server.
fn base(headers: &HeaderMap, app: &AppState) -> String {
    match headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
        Some(h) if !h.is_empty() && !h.contains(['/', '@', ' ']) => format!("http://{h}"),
        _ => format!("http://127.0.0.1:{}", app.config.dlna.port),
    }
}

/// The account DLNA works as.
fn user(app: &AppState) -> Option<User> {
    app.db.user_by_name(app.config.dlna.user.trim())
}

/// The request context of the DLNA user, for the playback handlers.
fn ctx(u: User, ip: &str, agent: &str) -> Ctx {
    Ctx { user: Some(u), token: None, lang: "en", ip: ip.to_string(), secure: false, user_agent: agent.to_string() }
}

/// The player in the activity view: its name from the user agent, or its address.
fn player(headers: &HeaderMap, ip: &str) -> (String, String) {
    let agent = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").trim();
    let id = format!("dlna-{}", &crate::auth::sha256(&format!("{ip}\0{agent}"))[..12]);
    let name: String = agent.split(['/', '(', ';']).next().unwrap_or("").trim().chars().take(40).collect();
    (id, if name.is_empty() { format!("DLNA ({ip})") } else { format!("DLNA – {name}") })
}

async fn description(State(app): State<Arc<AppState>>, Extension(info): Extension<Arc<Info>>) -> Response {
    xml_response(xml::description(&friendly_name(&app), &info.udn, crate::api::VERSION))
}

async fn icon() -> Response {
    static ICON: &[u8] = include_bytes!("../../../web/icons/icon-192.png");
    ([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "public, max-age=86400")], ICON).into_response()
}

fn soap_action(headers: &HeaderMap) -> String {
    let v = headers.get("soapaction").and_then(|v| v.to_str().ok()).unwrap_or("").trim().trim_matches('"');
    v.rsplit('#').next().unwrap_or("").to_string()
}

fn fault(code: u16, text: &str) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, [(header::CONTENT_TYPE, "text/xml; charset=\"utf-8\"")], xml::fault(code, text)).into_response()
}

/// Changes whenever entries are added or removed, so players reload their lists.
fn system_update_id(app: &AppState) -> u32 {
    let idx = app.catalog.snapshot();
    let (n, newest) = idx.all().fold((0u64, 0i64), |(n, m), it| (n + 1, m.max(it.added_at)));
    (n.wrapping_mul(2_654_435_761).wrapping_add((newest / 1000) as u64) & 0x7fff_ffff) as u32
}

/// ContentDirectory: Browse, the capabilities, Samsung's feature list and bookmarks.
async fn control_cd(
    State(app): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let action = soap_action(&headers);
    let Some(u) = user(&app) else { return fault(501, "DLNA user missing") };
    let a = |name: &str| xml::arg(&body, name).unwrap_or_default();
    match action.as_str() {
        "Browse" => {
            let id = a("ObjectID");
            let id = if id.is_empty() { "0".to_string() } else { id };
            let start = a("StartingIndex").parse().unwrap_or(0);
            let count = a("RequestedCount").parse().unwrap_or(0);
            let idx = app.catalog.snapshot();
            let tree = Tree::new(&app, &idx, &u, base(&headers, &app));
            let result = match a("BrowseFlag").as_str() {
                "BrowseMetadata" => tree.browse_metadata(&id).map(|d| (d, 1, 1)),
                "BrowseDirectChildren" => tree.browse_children(&id, start, count),
                _ => return fault(402, "Invalid BrowseFlag"),
            };
            match result {
                Some((didl, returned, total)) => xml_response(xml::soap(
                    xml::CD,
                    "Browse",
                    &[
                        ("Result", xml::esc(&didl)),
                        ("NumberReturned", returned.to_string()),
                        ("TotalMatches", total.to_string()),
                        ("UpdateID", system_update_id(&app).to_string()),
                    ],
                )),
                None => fault(701, "No such object"),
            }
        }
        "GetSearchCapabilities" => xml_response(xml::soap(xml::CD, &action, &[("SearchCaps", String::new())])),
        "GetSortCapabilities" => xml_response(xml::soap(xml::CD, &action, &[("SortCaps", String::new())])),
        "GetSystemUpdateID" => xml_response(xml::soap(xml::CD, &action, &[("Id", system_update_id(&app).to_string())])),
        // Samsung TVs ask which folders hold videos.
        "X_GetFeatureList" => {
            let features = r#"<Features xmlns="urn:schemas-upnp-org:av:avs" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="urn:schemas-upnp-org:av:avs http://www.upnp.org/schemas/av/avs.xsd"><Feature name="samsung.com_BASICVIEW" version="1"><container id="0" type="object.item.videoItem"/></Feature></Features>"#;
            xml_response(xml::soap(xml::CD, &action, &[("FeatureList", xml::esc(features))]))
        }
        // Samsung TVs save where playback stopped.
        "X_SetBookmark" => {
            let Some(id) = a("ObjectID").strip_prefix("item:").map(str::to_string) else { return fault(701, "No such object") };
            let seconds: i64 = a("PosSecond").parse().unwrap_or(0);
            let (client, device) = player(&headers, &addr.ip().to_string());
            let body = json!({"offsetMs": seconds * 1000, "client": client, "device": device, "state": "paused"});
            let c = ctx(u, &addr.ip().to_string(), "DLNA");
            let r = crate::api::playback::progress(State(app.clone()), Extension(c), Path(id), JsonBody(object(body))).await;
            if r.is_err() {
                return fault(701, "No such object");
            }
            xml_response(xml::soap(xml::CD, &action, &[]))
        }
        _ => fault(401, "Invalid Action"),
    }
}

/// The formats the server sends, for GetProtocolInfo.
fn source_protocols() -> String {
    [
        "video/x-matroska",
        "video/mp4",
        "video/quicktime",
        "video/x-msvideo",
        "video/mp2t",
        "video/webm",
        "video/x-ms-wmv",
        "video/mpeg",
        "text/srt",
    ]
    .iter()
    .map(|m| format!("http-get:*:{m}:*"))
    .collect::<Vec<_>>()
    .join(",")
}

async fn control_cm(headers: HeaderMap) -> Response {
    let action = soap_action(&headers);
    match action.as_str() {
        "GetProtocolInfo" => {
            xml_response(xml::soap(xml::CM, &action, &[("Source", xml::esc(&source_protocols())), ("Sink", String::new())]))
        }
        "GetCurrentConnectionIDs" => xml_response(xml::soap(xml::CM, &action, &[("ConnectionIDs", "0".into())])),
        "GetCurrentConnectionInfo" => xml_response(xml::soap(
            xml::CM,
            &action,
            &[
                ("RcsID", "-1".into()),
                ("AVTransportID", "-1".into()),
                ("ProtocolInfo", String::new()),
                ("PeerConnectionManager", String::new()),
                ("PeerConnectionID", "-1".into()),
                ("Direction", "Output".into()),
                ("Status", "OK".into()),
            ],
        )),
        _ => fault(401, "Invalid Action"),
    }
}

/// Event subscriptions: accepted, with the initial values sent to the subscriber; the server
/// sends no later events (players reload their lists through the update id instead).
async fn event(
    State(app): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(service): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    match method.as_str() {
        "SUBSCRIBE" => {
            let sid = headers
                .get("sid")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
                .unwrap_or_else(|| format!("uuid:{}", &crate::auth::sha256(&crate::auth::token())[..32]));
            let renewal = headers.contains_key("sid");
            // The callback must be the subscriber itself, so this cannot be used to send requests elsewhere.
            let callback = headers
                .get("callback")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().trim_start_matches('<').split('>').next().map(str::to_string))
                .and_then(|u| url::Url::parse(&u).ok())
                .filter(|u| u.scheme() == "http" && u.host_str().and_then(|h| h.parse::<std::net::IpAddr>().ok()) == Some(addr.ip()));
            if let (Some(url), false) = (callback, renewal) {
                let body = match service.as_str() {
                    "cd" => format!("<SystemUpdateID>{}</SystemUpdateID>", system_update_id(&app)),
                    _ => format!(
                        "<SourceProtocolInfo>{}</SourceProtocolInfo><SinkProtocolInfo></SinkProtocolInfo><CurrentConnectionIDs>0</CurrentConnectionIDs>",
                        xml::esc(&source_protocols())
                    ),
                };
                let sid = sid.clone();
                tokio::spawn(async move {
                    let body = format!(
                        r#"<?xml version="1.0" encoding="utf-8"?><e:propertyset xmlns:e="urn:schemas-upnp-org:event-1-0"><e:property>{body}</e:property></e:propertyset>"#
                    );
                    let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(5)).build() else { return };
                    let Ok(m) = reqwest::Method::from_bytes(b"NOTIFY") else { return };
                    let _ = client
                        .request(m, url)
                        .header("content-type", "text/xml; charset=\"utf-8\"")
                        .header("nt", "upnp:event")
                        .header("nts", "upnp:propchange")
                        .header("sid", sid)
                        .header("seq", "0")
                        .body(body)
                        .send()
                        .await;
                });
            }
            let mut r = StatusCode::OK.into_response();
            if let Ok(v) = HeaderValue::from_str(&sid) {
                r.headers_mut().insert("sid", v);
            }
            r.headers_mut().insert("timeout", HeaderValue::from_static("Second-1800"));
            r
        }
        "UNSUBSCRIBE" => StatusCode::OK.into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// A movie or episode the DLNA user may see.
fn visible(app: &AppState, u: &User, id: &str) -> Option<Arc<Item>> {
    let idx = app.catalog.snapshot();
    let it = idx.item(id)?;
    (may_see(&idx, u, it) && it.kind.playable() && it.file.is_some()).then(|| it.clone())
}

fn object(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    match v {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    }
}

/// `GET /dlna/media/{id}`: the file, with byte ranges and the DLNA headers.
async fn media(
    State(app): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let Some(u) = user(&app) else { return StatusCode::SERVICE_UNAVAILABLE.into_response() };
    let Some(it) = visible(&app, &u, &id) else { return StatusCode::NOT_FOUND.into_response() };
    let f = it.file.as_ref().expect("checked");
    let mut r = match crate::files::send(std::path::Path::new(&f.path), 0, &headers).await {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content::mime(f)));
    h.insert("transfermode.dlna.org", HeaderValue::from_static("Streaming"));
    h.insert("contentfeatures.dlna.org", HeaderValue::from_static(content::FLAGS));
    if headers.contains_key("getcaptioninfo.sec") {
        let idx = app.catalog.snapshot();
        let tree = Tree::new(&app, &idx, &u, base(&headers, &app));
        let subs = content::text_subtitles(f);
        if let Some((sid, _)) = tree.preferred_subtitle(&subs)
            && let Ok(v) = HeaderValue::from_str(&format!("{}/dlna/subtitle/{}/{sid}.srt", base(&headers, &app), it.id))
        {
            r.headers_mut().insert("captioninfo.sec", v);
        }
    }
    if method != Method::GET || !r.status().is_success() || f.size <= 0 {
        return r;
    }
    // Where the request starts in the file: `Content-Range: bytes 1000-1999/5000`.
    let offset = r
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("bytes ")?.split('-').next()?.parse::<u64>().ok())
        .unwrap_or(0);
    let ip = addr.ip().to_string();
    let (client, device) = player(&headers, &ip);
    let watch = Watch { app: app.clone(), user: u, item: it.clone(), client, device, ip, offset, sent: 0, last_report: None };
    let (parts, body) = r.into_parts();
    Response::from_parts(parts, Body::from_stream(Counted { inner: body.into_data_stream(), watch }))
}

/// A running stream: estimates the position from the bytes sent.
struct Watch {
    app: Arc<AppState>,
    user: User,
    item: Arc<Item>,
    client: String,
    device: String,
    ip: String,
    offset: u64,
    sent: u64,
    last_report: Option<Instant>,
}

impl Watch {
    fn position_ms(&self) -> i64 {
        let f = self.item.file.as_ref().expect("checked");
        let share = (self.offset + self.sent) as f64 / f.size.max(1) as f64;
        ((share.min(1.0) * f.duration_ms as f64) as i64 - BUFFER_MS).max(0)
    }

    fn watching(&self) -> bool {
        let size = self.item.file.as_ref().map_or(0, |f| u64::try_from(f.size).unwrap_or(0));
        self.sent >= WATCHING_BYTES.min(size / 20).max(1)
    }

    /// Announces the playback, then keeps it current in the activity view.
    fn report(&mut self) {
        let first = self.last_report.is_none();
        if !first && self.last_report.is_some_and(|t| t.elapsed() < REPORT_EVERY) {
            return;
        }
        if !self.watching() {
            return;
        }
        self.last_report = Some(Instant::now());
        let (app, item_id, pos) = (self.app.clone(), self.item.id.clone(), self.position_ms());
        if first {
            let body = json!({"player": "external", "client": self.client, "device": self.device, "startMs": pos});
            let c = ctx(self.user.clone(), &self.ip, &self.device);
            tokio::spawn(async move {
                let _ = crate::api::playback::start(State(app), Extension(c), Path(item_id), JsonBody(object(body))).await;
            });
        } else {
            app.activity.progress(&self.user.id, &item_id, Some(&self.client), pos, "playing", false);
        }
    }
}

impl Drop for Watch {
    /// The player stopped (or seeks, with a new request): the position becomes the resume position.
    fn drop(&mut self) {
        if !self.watching() || tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let pos = self.position_ms();
        let body = json!({"offsetMs": pos, "client": self.client, "device": self.device, "state": "playing", "stopped": true});
        let (app, item_id, c) = (self.app.clone(), self.item.id.clone(), ctx(self.user.clone(), &self.ip, &self.device));
        tokio::spawn(async move {
            let _ = crate::api::playback::progress(State(app), Extension(c), Path(item_id), JsonBody(object(body))).await;
        });
    }
}

/// The body of a stream, counting what it sends.
struct Counted<S> {
    inner: S,
    watch: Watch,
}

impl<S: Stream<Item = Result<Bytes, axum::Error>> + Unpin> Stream for Counted<S> {
    type Item = Result<Bytes, axum::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Option<Self::Item>> {
        let r = Pin::new(&mut self.inner).poll_next(cx);
        if let Poll::Ready(Some(Ok(b))) = &r {
            self.watch.sent += b.len() as u64;
            self.watch.report();
        }
        r
    }
}

/// `GET /dlna/image/{id}`: poster, or the picture of an episode.
async fn image(State(app): State<Arc<AppState>>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    let Some(u) = user(&app) else { return StatusCode::SERVICE_UNAVAILABLE.into_response() };
    let it = {
        let idx = app.catalog.snapshot();
        match idx.item(&id) {
            Some(it) if may_see(&idx, &u, it) => it.clone(),
            _ => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    let kind = if it.kind == crate::media::Kind::Episode { "thumb" } else { "poster" };
    match image_path(&app, &it, kind).await {
        Some(p) => crate::files::send(&p, IMAGE_CACHE_SECONDS, &headers).await.unwrap_or_else(IntoResponse::into_response),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `GET /dlna/subtitle/{id}/{sid}.srt`: a text subtitle as SubRip.
async fn subtitle(State(app): State<Arc<AppState>>, Path((id, file)): Path<(String, String)>) -> Response {
    let Some(u) = user(&app) else { return StatusCode::SERVICE_UNAVAILABLE.into_response() };
    let Some(it) = visible(&app, &u, &id) else { return StatusCode::NOT_FOUND.into_response() };
    let sid = file.strip_suffix(".srt").unwrap_or(&file);
    let valid = sid.len() > 1 && sid.starts_with(['e', 'x']) && sid[1..].bytes().all(|b| b.is_ascii_digit());
    if !valid {
        return StatusCode::NOT_FOUND.into_response();
    }
    match app.subtitles.vtt(&it, sid).await {
        Some(vtt) => ([(header::CONTENT_TYPE, "text/srt; charset=utf-8")], crate::media::subtitles::vtt_to_srt(&vtt)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
