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

//! Sending files from disk (artwork, videos, subtitles): content type, `ETag` and
//! `Last-Modified`, `304 Not Modified` and single byte ranges (`206 Partial Content`), streamed
//! without reading the file into memory.

use crate::http::ApiError;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::io::SeekFrom;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

const BUFFER: usize = 256 * 1024;

/// Content type by file extension.
pub fn content_type(file_name: &str) -> &'static str {
    let ext = file_name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "ts" | "m2ts" | "mts" => "video/mp2t",
        "wmv" => "video/x-ms-wmv",
        "mpg" | "mpeg" => "video/mpeg",
        "flv" => "video/x-flv",
        "ogv" => "video/ogg",
        "3gp" => "video/3gpp",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "vtt" => "text/vtt; charset=utf-8",
        "srt" | "txt" => "text/plain; charset=utf-8",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

/// Sends a file. `max_age_s > 0` lets the browser cache it (artwork); videos use 0.
pub async fn send(path: &Path, max_age_s: u32, request: &HeaderMap) -> Result<Response, ApiError> {
    let meta = tokio::fs::metadata(path).await.map_err(|_| ApiError::not_found())?;
    if !meta.is_file() {
        return Err(ApiError::not_found());
    }
    let size = meta.len();
    let modified = meta.modified().unwrap_or(UNIX_EPOCH);
    let modified_ms = modified.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_millis();
    let etag = format!("\"{size:x}-{modified_ms:x}\"");

    let mut h = HeaderMap::new();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type(&name)));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(header::ETAG, HeaderValue::from_str(&etag).expect("ascii"));
    h.insert(header::LAST_MODIFIED, HeaderValue::from_str(&httpdate::fmt_http_date(modified)).expect("ascii"));
    let cache = if max_age_s > 0 { format!("private, max-age={max_age_s}") } else { "no-cache".into() };
    h.insert(header::CACHE_CONTROL, HeaderValue::from_str(&cache).expect("ascii"));

    if request.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return Ok((StatusCode::NOT_MODIFIED, h).into_response());
    }

    let (mut start, mut end, mut status) = (0u64, size.saturating_sub(1), StatusCode::OK);
    if let Some(range) = request.get(header::RANGE).and_then(|v| v.to_str().ok())
        && size > 0
    {
        match parse_range(range, size) {
            Some(Ok((s, e))) => {
                (start, end, status) = (s, e, StatusCode::PARTIAL_CONTENT);
                h.insert(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes {s}-{e}/{size}")).expect("ascii"));
            }
            Some(Err(())) => {
                h.insert(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes */{size}")).expect("ascii"));
                return Ok((StatusCode::RANGE_NOT_SATISFIABLE, h).into_response());
            }
            None => {}
        }
    }
    let length = if size == 0 { 0 } else { end - start + 1 };
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    if length == 0 {
        return Ok((status, h).into_response());
    }
    let mut file = tokio::fs::File::open(path).await.map_err(|_| ApiError::not_found())?;
    if start > 0 {
        file.seek(SeekFrom::Start(start)).await.map_err(|_| ApiError::not_found())?;
    }
    let body = Body::from_stream(ReaderStream::with_capacity(file.take(length), BUFFER));
    Ok((status, h, body).into_response())
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n`. None: not a usable single range (send the whole file);
/// Err: outside the file.
fn parse_range(v: &str, size: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = v.trim().strip_prefix("bytes=")?;
    // Multiple ranges ("bytes=0-1,5-9") are not supported; browsers never ask for them for media.
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    let (start, end) = match (a.is_empty(), b.is_empty()) {
        (true, true) => return None,
        (true, false) => {
            let suffix: u64 = b.parse().ok()?;
            (size.saturating_sub(suffix), size - 1)
        }
        (false, _) => {
            let start: u64 = a.parse().ok()?;
            let end = if b.is_empty() { size - 1 } else { b.parse::<u64>().ok()?.min(size - 1) };
            (start, end)
        }
    };
    if start >= size || start > end { Some(Err(())) } else { Some(Ok((start, end))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some(Ok((0, 99))));
        assert_eq!(parse_range("bytes=500-", 1000), Some(Ok((500, 999))));
        assert_eq!(parse_range("bytes=-100", 1000), Some(Ok((900, 999))));
        assert_eq!(parse_range("bytes=900-5000", 1000), Some(Ok((900, 999))));
        assert_eq!(parse_range("bytes=1000-", 1000), Some(Err(())));
        assert_eq!(parse_range("bytes=0-1,5-9", 1000), None);
        assert_eq!(parse_range("items=0-1", 1000), None);
    }
}
