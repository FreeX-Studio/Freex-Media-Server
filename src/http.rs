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

//! The web server: request context, errors, security rules and FreeX Web.
//!
//! Every request passes [`context`]: it adds the security headers, blocks state-changing requests
//! from other sites (CSRF), resolves the signed-in user and the language, and turns
//! [`ApiError`]s into JSON answers in that language. Handlers get the result as [`Ctx`].

use crate::AppState;
use crate::db::User;
use crate::i18n::{self, Text};
use axum::Json;
use axum::body::Body;
use axum::extract::{ConnectInfo, FromRequest, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::sync::Arc;

pub const TOKEN_COOKIE: &str = "freex_token";
pub const TOKEN_HEADER: &str = "x-freex-token";
/// Largest JSON body accepted.
pub const MAX_BODY: usize = 1024 * 1024;

const CSP: &str = "default-src 'self'; script-src 'self' https://www.gstatic.com; style-src 'self'; \
    img-src 'self' data: blob:; media-src 'self' blob:; worker-src 'self' blob:; connect-src 'self'; \
    font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; \
    frame-src https://www.youtube-nocookie.com";

/// What a handler knows about the request.
#[derive(Clone)]
pub struct Ctx {
    pub user: Option<User>,
    pub token: Option<String>,
    pub lang: &'static str,
    pub ip: String,
    pub secure: bool,
    pub user_agent: String,
}

impl Ctx {
    /// The signed-in user, or 401.
    pub fn user(&self) -> Result<&User, ApiError> {
        self.user.as_ref().ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "error.notSignedIn"))
    }

    /// The signed-in admin, or 401/403.
    pub fn admin(&self) -> Result<&User, ApiError> {
        let u = self.user()?;
        if u.admin { Ok(u) } else { Err(ApiError::new(StatusCode::FORBIDDEN, "error.adminOnly")) }
    }

    pub fn t(&self, text: &Text) -> String {
        text.render(self.lang)
    }
}

/// An error answer: status and a message, translated when it is sent.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub text: Text,
    pub retry_after: Option<u64>,
}

impl ApiError {
    pub fn new(status: StatusCode, key: &str) -> Self {
        ApiError { status, text: Text::new(key), retry_after: None }
    }

    pub fn from_text(status: StatusCode, text: Text) -> Self {
        ApiError { status, text, retry_after: None }
    }

    pub fn bad_request(key: &str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, key)
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "error.notFound")
    }

    pub fn arg(mut self, a: impl Into<i18n::Arg>) -> Self {
        self.text = self.text.arg(a);
        self
    }
}

/// Marks a response as an error to be written by [`context`] in the request's language.
#[derive(Clone)]
struct PendingError(Text);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut r = self.status.into_response();
        if let Some(s) = self.retry_after {
            r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(s));
        }
        r.extensions_mut().insert(PendingError(self.text));
        r
    }
}

pub type ApiResult<T = Response> = Result<T, ApiError>;

pub fn ok(v: Value) -> ApiResult {
    Ok(Json(v).into_response())
}

pub fn created(v: Value) -> ApiResult {
    Ok((StatusCode::CREATED, Json(v)).into_response())
}

pub fn no_content() -> ApiResult {
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// A JSON object as request body; an empty body counts as `{}`.
pub struct JsonBody(pub serde_json::Map<String, Value>);

impl<S: Send + Sync> FromRequest<S> for JsonBody {
    type Rejection = ApiError;

    async fn from_request(req: Request, _: &S) -> Result<Self, ApiError> {
        let bytes = axum::body::to_bytes(req.into_body(), MAX_BODY)
            .await
            .map_err(|_| ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "error.requestTooLarge"))?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(JsonBody(serde_json::Map::new()));
        }
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(m)) => Ok(JsonBody(m)),
            Ok(_) => Err(ApiError::bad_request("error.invalidJson").arg("expected an object")),
            Err(e) => Err(ApiError::bad_request("error.invalidJson").arg(e.to_string())),
        }
    }
}

impl JsonBody {
    pub fn str(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str)
    }

    pub fn string(&self, key: &str) -> String {
        self.str(key).unwrap_or("").to_string()
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.0.get(key).and_then(Value::as_bool)
    }

    pub fn has(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }
}

// ------------------------------------------------------------------ middleware

/// Security headers, CSRF protection, user and language, error bodies. See the module docs.
pub async fn context(State(app): State<Arc<AppState>>, mut req: Request, next: Next) -> Response {
    let headers = req.headers().clone();
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip().to_string()).unwrap_or_else(|| "?".into());
    let trust = app.config.server.trust_proxy;
    let ip = if trust { forwarded_for(&headers).unwrap_or(peer) } else { peer };
    let secure = app.tls || (trust && header_str(&headers, "x-forwarded-proto").is_some_and(|p| p.eq_ignore_ascii_case("https")));
    let method = req.method().clone();
    let safe = method == Method::GET || method == Method::HEAD;
    let api = req.uri().path().starts_with("/api/");

    let mut ctx = Ctx {
        user: None,
        token: None,
        lang: i18n::negotiate(header_str(&headers, "accept-language")),
        ip,
        secure,
        user_agent: header_str(&headers, "user-agent").unwrap_or("").to_string(),
    };

    // HTTP/2 carries the host in the :authority pseudo-header, which is part of the URI here.
    let authority = req.uri().authority().map(|a| a.to_string());
    let response = if !safe && !same_origin(&headers, authority.as_deref(), trust) {
        ApiError::new(StatusCode::FORBIDDEN, "error.crossSite").into_response()
    } else {
        if api {
            // The cookie only counts for reading (images, video), so other sites cannot act for a user.
            let token = header_str(&headers, TOKEN_HEADER)
                .or_else(|| header_str(&headers, "authorization").and_then(|a| a.strip_prefix("Bearer ")).map(str::trim))
                .map(str::to_string)
                .or_else(|| if safe { cookie(&headers, TOKEN_COOKIE) } else { None });
            if let Some(t) = token.filter(|t| !t.is_empty() && t.len() < 200) {
                let max_idle = i64::from(app.config.security.session_days) * 86_400_000;
                if let Some(u) = app.db.user_for_token(&t, max_idle, &ctx.ip) {
                    if i18n::supported(&u.ui_language) {
                        ctx.lang = static_lang(&u.ui_language);
                    }
                    ctx.user = Some(u);
                    ctx.token = Some(t);
                }
            }
        }
        req.extensions_mut().insert(ctx.clone());
        next.run(req).await
    };

    // Body limits of axum answer in plain text; the API answers in its own error format.
    let response = if api && response.status() == StatusCode::PAYLOAD_TOO_LARGE && response.extensions().get::<PendingError>().is_none() {
        ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "error.requestTooLarge").into_response()
    } else {
        response
    };
    let mut response = match response.extensions().get::<PendingError>().cloned() {
        Some(PendingError(text)) => {
            let status = response.status();
            let mut r =
                (status, Json(json!({"error": text.render(ctx.lang), "code": text.key, "status": status.as_u16()}))).into_response();
            if let Some(v) = response.headers().get(header::RETRY_AFTER) {
                r.headers_mut().insert(header::RETRY_AFTER, v.clone());
            }
            r
        }
        None => response,
    };
    add_security_headers(response.headers_mut(), secure, &app.build);
    response
}

fn static_lang(lang: &str) -> &'static str {
    i18n::LANGUAGES.iter().find(|l| **l == lang).copied().unwrap_or("en")
}

fn add_security_headers(h: &mut HeaderMap, secure: bool, build: &str) {
    let set = |h: &mut HeaderMap, k: &'static str, v: &str| {
        if let Ok(v) = HeaderValue::from_str(v) {
            h.entry(k).or_insert(v);
        }
    };
    set(h, "x-content-type-options", "nosniff");
    set(h, "referrer-policy", "same-origin");
    set(h, "x-frame-options", "DENY");
    set(h, "cross-origin-opener-policy", "same-origin");
    set(h, "cross-origin-resource-policy", "same-origin");
    set(h, "permissions-policy", "camera=(), microphone=(), geolocation=(), payment=()");
    set(h, "content-security-policy", CSP);
    if secure {
        set(h, "strict-transport-security", "max-age=31536000");
    }
    // Lets an open FreeX Web notice that the server now has a newer one.
    set(h, "x-freex-build", build);
}

fn header_str<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

fn forwarded_for(h: &HeaderMap) -> Option<String> {
    header_str(h, "x-forwarded-for").and_then(|v| v.split(',').next()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn cookie(h: &HeaderMap, name: &str) -> Option<String> {
    h.get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

/// A request that changes something must come from our own pages, or without `Origin` (native apps).
fn same_origin(h: &HeaderMap, authority: Option<&str>, trust_proxy: bool) -> bool {
    let Some(origin) = header_str(h, "origin") else { return true };
    let host = if trust_proxy { header_str(h, "x-forwarded-host") } else { None }.or_else(|| header_str(h, "host")).or(authority);
    let (Some(host), Ok(o)) = (host, url::Url::parse(origin)) else { return false };
    let Some(oh) = o.host_str() else { return false };
    let with_port = match o.port() {
        Some(p) => format!("{oh}:{p}"),
        None => oh.to_string(),
    };
    with_port.eq_ignore_ascii_case(host.trim()) || oh.eq_ignore_ascii_case(host.trim())
}

/// Sets the sign-in cookie on a response.
pub fn session_cookie(r: &mut Response, token: &str, max_age_s: i64, secure: bool) {
    let v =
        format!("{TOKEN_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age_s}{}", if secure { "; Secure" } else { "" });
    if let Ok(v) = HeaderValue::from_str(&v) {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
}

// ------------------------------------------------------------------ FreeX Web

#[derive(RustEmbed)]
#[folder = "web/"]
struct Web;

/// FreeX Web as served: the build id and `index.html` with versioned asset URLs.
pub struct WebBundle {
    pub build: String,
    index: Vec<u8>,
    pub version: Value,
}

impl WebBundle {
    /// Hashes `index.html` and the scripts and styles it loads into a build id, and adds that id
    /// to their URLs, so browsers can cache them for good and still get every update at once.
    pub fn load() -> WebBundle {
        let html = Web::get("index.html").map(|f| String::from_utf8_lossy(&f.data).into_owned()).unwrap_or_default();
        let mut hash = Sha256::new();
        hash.update(html.as_bytes());
        let assets = asset_urls(&html);
        for a in &assets {
            if let Some(f) = Web::get(a) {
                hash.update(&f.data);
            }
        }
        let build = hex::encode(&hash.finalize()[..6]);
        let mut index = html.replacen("<head>", &format!("<head>\n    <meta name=\"freex-build\" content=\"{build}\">"), 1);
        for a in &assets {
            for attr in ["src", "href"] {
                index = index.replace(&format!("{attr}=\"/{a}\""), &format!("{attr}=\"/{a}?v={build}\""));
            }
        }
        let version = Web::get("version.json")
            .and_then(|f| serde_json::from_slice(&f.data).ok())
            .unwrap_or_else(|| json!({"name": "FreeX Web", "version": "?"}));
        WebBundle { build, index: index.into_bytes(), version }
    }
}

/// Local `.js` and `.css` files referenced as `src="/..."` or `href="/..."`.
fn asset_urls(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for attr in ["src=\"/", "href=\"/"] {
        let mut rest = html;
        while let Some(i) = rest.find(attr) {
            rest = &rest[i + attr.len()..];
            let end = rest.find('"').unwrap_or(0);
            let url = &rest[..end];
            let ok = url.chars().all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c));
            if ok && (url.ends_with(".js") || url.ends_with(".css")) && !out.iter().any(|o| o == url) {
                out.push(url.to_string());
            }
        }
    }
    out
}

/// Serves FreeX Web; paths without a file extension are routes of the app and get `index.html`.
pub async fn web(State(app): State<Arc<AppState>>, method: Method, uri: Uri) -> Response {
    if uri.path().starts_with("/api/") {
        return ApiError::new(StatusCode::NOT_FOUND, "error.unknownEndpoint").into_response();
    }
    if method != Method::GET && method != Method::HEAD {
        return ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "error.methodNotAllowed").into_response();
    }
    let path = uri.path().trim_start_matches('/');
    if path.contains("..") || path.contains('\\') || path.contains("//") {
        return ApiError::not_found().into_response();
    }
    let file = if path.is_empty() { "index.html" } else { path };
    let index = || {
        let mut r = Response::new(Body::from(app.web.index.clone()));
        r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
        r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        r
    };
    if file == "index.html" {
        return index();
    }
    match Web::get(file) {
        Some(f) => {
            let versioned = uri.query().is_some_and(|q| q.split('&').any(|p| p == format!("v={}", app.build)));
            let mime = mime_guess::from_path(file).first_or_octet_stream();
            let mut r = Response::new(Body::from(f.data.into_owned()));
            let ct = if mime.type_() == "text" || mime.subtype() == "javascript" || mime.subtype() == "json" {
                format!("{mime}; charset=utf-8")
            } else {
                mime.to_string()
            };
            r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_str(&ct).unwrap());
            r.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static(if versioned { "public, max-age=31536000, immutable" } else { "public, max-age=300" }),
            );
            r
        }
        None if file.contains('.') => ApiError::not_found().into_response(),
        None => index(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_check() {
        let mut h = HeaderMap::new();
        h.insert("host", HeaderValue::from_static("media.example.org:32500"));
        assert!(same_origin(&h, None, false), "no Origin: native app");
        h.insert("origin", HeaderValue::from_static("http://media.example.org:32500"));
        assert!(same_origin(&h, None, false));
        h.insert("origin", HeaderValue::from_static("https://evil.example"));
        assert!(!same_origin(&h, None, false));
        h.insert("origin", HeaderValue::from_static("null"));
        assert!(!same_origin(&h, None, false));
    }

    #[test]
    fn web_bundle_versions_assets() {
        let w = WebBundle::load();
        let index = String::from_utf8(w.index).unwrap();
        assert!(index.contains(&format!("/core.js?v={}", w.build)));
        assert!(index.contains("name=\"freex-build\""));
    }
}
