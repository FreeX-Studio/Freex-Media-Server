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

//! Playback for the apps. `PlaybackInfo` reads the app's device profile and decides with the
//! FreeX playback decision whether the file plays directly; otherwise it hands out an HLS URL.
//! That playlist starts the FreeX conversion only when the player loads it, and its segments
//! carry a playback grant in the path, so native players need no sign-in header for them.
//! Position reports go to the FreeX watch state, activity list and webhooks.

use super::dto::{self, Dto, audio_position, media_source, ms, subtitle_codec, subtitle_id, subtitle_is_text};
use super::{Cx, Id, Out, empty, item_guid, json_out, not_found, parse_guid};
use crate::api::media::visible_item;
use crate::auth::now_ms;
use crate::http::JsonBody;
use crate::media::playback::{self, Capabilities, Mode, QUALITIES, codec_kind, container_kind};
use crate::media::{Item, Kind, MediaFile};
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use std::sync::Arc;

/// How long the choices of a playback info are kept for its playlist and position reports.
const PLAY_TTL_MS: i64 = 24 * 3_600_000;
/// More playbacks than this at once are not expected; the oldest are dropped.
const MAX_PLAYS: usize = 500;

/// A playback, from its playback info until the player stops.
#[derive(Clone, Debug)]
pub struct Play {
    user_id: String,
    /// The entry with the file to play (a version of a movie has its own id).
    item_id: String,
    /// Capabilities for the FreeX decision, see [`crate::media::playback::Capabilities`].
    caps: Value,
    quality: String,
    audio: i32,
    subtitle: Option<String>,
    burn: bool,
    start_ms: i64,
    direct: bool,
    /// Set once the playlist was loaded: the grant of its files and the conversion session.
    grant: Option<String>,
    session: Option<String>,
    created: i64,
    /// The app that asked for the playback info. Players load the playlist with just the token
    /// in the URL, so later requests of the same playback count for this app and device.
    client_id: String,
    device: String,
}

// ====================================================================== device profile

fn split(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_str)
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .map(|s| match s.as_str() {
            "h265" | "hvc1" | "hev1" => "hevc".to_string(),
            "avc" | "avc1" => "h264".to_string(),
            "matroska" => "mkv".to_string(),
            "m4v" | "mov" => "mp4".to_string(),
            _ => s,
        })
        .collect()
}

/// A field of a JSON object without regard to case (the apps send PascalCase or camelCase).
fn get<'v>(v: &'v Value, key: &str) -> Option<&'v Value> {
    v.as_object()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
}

fn list<'v>(v: &'v Value, key: &str) -> &'v [Value] {
    get(v, key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn is_video(p: &Value) -> bool {
    get(p, "Type").and_then(Value::as_str).is_none_or(|t| t.eq_ignore_ascii_case("video"))
}

/// What we know of the file for the conditions of codec profiles.
fn property(f: &MediaFile, audio: Option<&crate::media::AudioStream>, name: &str) -> Option<String> {
    let bits = if f.pixel_format.contains("12") {
        12
    } else if f.pixel_format.contains("10") {
        10
    } else {
        8
    };
    let range = dto::range_type(f);
    Some(match name.to_ascii_lowercase().as_str() {
        "width" => f.width.to_string(),
        "height" => f.height.to_string(),
        "videobitdepth" => bits.to_string(),
        "videoprofile" if !f.video_profile.is_empty() => f.video_profile.to_lowercase(),
        "videorangetype" => range.to_lowercase(),
        "videobitrate" if f.bitrate > 0 => f.bitrate.to_string(),
        "isinterlaced" | "isanamorphic" => "false".into(),
        "isavc" => (codec_kind(&f.video_codec) == "h264").to_string(),
        "audiochannels" => audio?.channels.to_string(),
        "audiobitrate" if audio?.bitrate > 0 => audio?.bitrate.to_string(),
        "issecondaryaudio" => "false".into(),
        _ => return None,
    })
}

/// Whether a condition of a codec profile holds; unknown properties pass.
fn condition_ok(c: &Value, f: &MediaFile, audio: Option<&crate::media::AudioStream>) -> bool {
    let (Some(prop), Some(cond)) = (get(c, "Property").and_then(Value::as_str), get(c, "Condition").and_then(Value::as_str)) else {
        return true;
    };
    let Some(have) = property(f, audio, prop) else { return true };
    let want = get(c, "Value").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default().to_lowercase();
    let num = |s: &str| s.parse::<f64>().ok();
    match cond.to_ascii_lowercase().as_str() {
        "equals" => have == want,
        "notequals" => have != want,
        "equalsany" => want.split('|').any(|w| w.trim() == have),
        "lessthanequal" => matches!((num(&have), num(&want)), (Some(a), Some(b)) if a <= b),
        "greaterthanequal" => matches!((num(&have), num(&want)), (Some(a), Some(b)) if a >= b),
        _ => true,
    }
}

/// Whether the codec profiles of the device allow a codec (`kind` "Video" or "VideoAudio").
fn codec_allowed(profile: &Value, kind: &str, codec: &str, f: &MediaFile, audio: Option<&crate::media::AudioStream>) -> bool {
    list(profile, "CodecProfiles").iter().all(|cp| {
        let t = get(cp, "Type").and_then(Value::as_str).unwrap_or("");
        let codecs = split(get(cp, "Codec"));
        if !t.eq_ignore_ascii_case(kind) || !(codecs.is_empty() || codecs.iter().any(|c| c == codec)) {
            return true;
        }
        // Apply conditions decide whether the profile applies at all.
        if !list(cp, "ApplyConditions").iter().all(|c| condition_ok(c, f, audio)) {
            return true;
        }
        list(cp, "Conditions").iter().all(|c| condition_ok(c, f, audio))
    })
}

/// Translates a Jellyfin device profile into the capabilities of the FreeX decision for one
/// file and audio track. Direct play is offered when one direct play profile takes container,
/// video and audio of the file together; the transcoding profile says which codecs may be copied
/// into the HLS stream. Without a profile the app plays the file as it is.
pub fn capabilities(profile: Option<&Value>, f: &MediaFile, audio_pos: i32) -> Value {
    let audio = usize::try_from(playback::choose_audio(f, audio_pos)).ok().and_then(|i| f.audio_streams.get(i));
    let container = container_kind(f);
    let video = codec_kind(&f.video_codec);
    let audio_codec = audio.map(|a| codec_kind(&a.codec));
    let all_audio: Vec<String> = f.audio_streams.iter().map(|a| codec_kind(&a.codec)).collect();
    let Some(profile) = profile.filter(|p| !list(p, "DirectPlayProfiles").is_empty() || !list(p, "TranscodingProfiles").is_empty()) else {
        return json!({"containers": [container], "video": [video], "audio": all_audio, "mseVideo": [video], "mseAudio": all_audio, "directPlay": true});
    };
    let video_ok = codec_allowed(profile, "Video", &video, f, audio);
    let audio_ok = audio_codec.as_ref().is_none_or(|c| codec_allowed(profile, "VideoAudio", c, f, audio));
    let direct = video_ok
        && audio_ok
        && list(profile, "DirectPlayProfiles").iter().filter(|p| is_video(p)).any(|p| {
            let (cs, vs, aus) = (split(get(p, "Container")), split(get(p, "VideoCodec")), split(get(p, "AudioCodec")));
            let ext = f.container.to_lowercase();
            (cs.is_empty() || cs.contains(&container) || cs.contains(&ext))
                && (vs.is_empty() || vs.contains(&video))
                && (aus.is_empty() || audio_codec.as_ref().is_none_or(|a| aus.contains(a)))
        });
    let hls = list(profile, "TranscodingProfiles")
        .iter()
        .filter(|p| is_video(p))
        .find(|p| get(p, "Protocol").and_then(Value::as_str).is_some_and(|x| x.eq_ignore_ascii_case("hls")))
        .or_else(|| list(profile, "TranscodingProfiles").iter().find(|p| is_video(p)));
    let (mut mse_video, mut mse_audio) = match hls {
        Some(p) => (split(get(p, "VideoCodec")), split(get(p, "AudioCodec"))),
        None => (vec!["h264".into()], vec!["aac".into()]),
    };
    if mse_video.is_empty() {
        mse_video.push("h264".into());
    }
    if mse_audio.is_empty() {
        mse_audio.push("aac".into());
    }
    if !video_ok {
        mse_video.retain(|c| *c != video);
    }
    if !audio_ok && let Some(a) = &audio_codec {
        mse_audio.retain(|c| c != a);
    }
    // The FreeX decision plays directly exactly when the file's own formats are listed.
    let (containers, videos, audios) = if direct { (vec![container], vec![video], all_audio) } else { (vec![], vec![], vec![]) };
    json!({"containers": containers, "video": videos, "audio": audios, "mseVideo": mse_video, "mseAudio": mse_audio, "directPlay": direct})
}

/// Whether the device shows a bitmap subtitle itself (method `Embed`), so it is not burned in.
fn embeds_subtitle(profile: Option<&Value>, codec: &str) -> bool {
    let codec = subtitle_codec(codec);
    profile.is_some_and(|p| {
        list(p, "SubtitleProfiles").iter().any(|s| {
            get(s, "Format").and_then(Value::as_str).is_some_and(|x| x.eq_ignore_ascii_case(&codec))
                && get(s, "Method").and_then(Value::as_str).is_some_and(|m| m.eq_ignore_ascii_case("embed"))
        })
    })
}

/// The FreeX quality for the bitrate limit of the app: the original if the file fits.
fn quality_for(max_bitrate: Option<i64>, f: &MediaFile) -> String {
    let Some(max) = max_bitrate.filter(|m| *m > 0) else { return "auto".into() };
    if f.bitrate > 0 && f.bitrate <= max {
        return "original".into();
    }
    QUALITIES[1..].iter().find(|q| q.video_bitrate <= max * 85 / 100).map_or("1000", |q| q.id).to_string()
}

// ====================================================================== playback info

/// The entry with the file to play: the version named by `MediaSourceId`, or the entry itself.
fn file_item(idx: &crate::media::catalog::Index, it: &Arc<Item>, media_source: Option<String>) -> Arc<Item> {
    match media_source.as_deref().and_then(parse_guid) {
        Some(Id::Item(v)) => idx.item(&v).filter(|x| x.key() == it.key()).cloned().unwrap_or_else(|| it.clone()),
        _ => it.clone(),
    }
}

fn new_id() -> String {
    crate::auth::sha256(&crate::auth::token())[..32].to_string()
}

/// `GET|POST /Items/{id}/PlaybackInfo`.
pub fn playback_info(cx: &Cx, raw: &str) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let Some(Id::Item(id)) = parse_guid(raw) else { return Err(not_found()) };
    let it = visible_item(&idx, u, &id).map_err(|_| not_found())?;
    if !it.kind.playable() {
        return Err(StatusCode::BAD_REQUEST.into());
    }
    let r = &cx.req;
    let fi = file_item(&idx, &it, r.param("MediaSourceId"));
    let f = fi.file.as_ref().ok_or_else(not_found)?;
    let profile = r.field("DeviceProfile");
    let audio = r.int("AudioStreamIndex").map_or(-1, |i| audio_position(f, i as i32));
    let sub_index = r.int("SubtitleStreamIndex").filter(|i| *i >= 0).map(|i| i as i32);
    let subtitle = sub_index.and_then(|i| subtitle_id(f, i));
    // Bitmap subtitles the device cannot show are burned into the picture.
    let burn = sub_index.is_some_and(|i| {
        !subtitle_is_text(f, i)
            && !f.subtitle_streams.iter().find(|s| s.stream_index == i).is_some_and(|s| embeds_subtitle(profile, &s.codec))
    });
    let mut caps = capabilities(profile, f, audio);
    let allow_direct = r.bool("EnableDirectPlay").unwrap_or(true);
    if !allow_direct {
        caps["directPlay"] = json!(false);
    }
    let quality = quality_for(r.int("MaxStreamingBitrate"), f);
    let start_ms = r.int("StartTimeTicks").map_or(0, ms).max(0);

    let home = playback::home_network(&cx.ctx.ip);
    let saved = if home { &u.quality_home } else { &u.quality_away };
    let wanted = if quality == "auto" { saved.as_str() } else { quality.as_str() };
    let q = if wanted == "auto" || !playback::known_quality(wanted) {
        playback::automatic(home, 0, f.bitrate)
    } else {
        playback::quality(wanted)
    };
    let plan = playback::plan(f, &Capabilities::parse(&caps), q, audio, subtitle.as_deref().filter(|_| burn), cx.app.transcoder.is_some());
    let direct = plan.mode == Mode::Direct;

    let token = cx.token();
    let item_id = item_guid(&it.id);
    let mut source = media_source(&item_id, &fi, "", Some(token)).ok_or_else(not_found)?;
    let play_session = new_id();
    source["SupportsDirectPlay"] = json!(direct);
    source["SupportsDirectStream"] = json!(false);
    source["SupportsTranscoding"] = json!(cx.app.transcoder.is_some());
    if let Some(i) = sub_index {
        source["DefaultSubtitleStreamIndex"] = json!(i);
    }
    if let Some(i) = r.int("AudioStreamIndex") {
        source["DefaultAudioStreamIndex"] = json!(i);
    }
    if !direct {
        let url = format!("/videos/{item_id}/master.m3u8?PlaySessionId={play_session}&MediaSourceId={}&ApiKey={token}", item_guid(&fi.id));
        source["TranscodingUrl"] = json!(url);
        source["TranscodingSubProtocol"] = json!("hls");
        source["TranscodingContainer"] = json!("mp4");
        // Burned-in subtitles are part of the picture.
        if burn && let Some(streams) = source["MediaStreams"].as_array_mut() {
            for s in streams.iter_mut().filter(|s| s["Index"] == json!(sub_index)) {
                s["DeliveryMethod"] = json!("Encode");
            }
        }
    }

    let play = Play {
        user_id: u.id.clone(),
        item_id: fi.id.clone(),
        caps,
        quality: q.id.to_string(),
        audio,
        subtitle,
        burn,
        start_ms,
        direct,
        grant: None,
        session: None,
        created: now_ms(),
        client_id: cx.req.client.id(),
        device: cx.req.client.label(),
    };
    {
        let mut plays = cx.jf().plays.lock().unwrap();
        let now = now_ms();
        plays.retain(|_, p| now - p.created < PLAY_TTL_MS);
        if plays.len() >= MAX_PLAYS
            && let Some(oldest) = plays.iter().min_by_key(|(_, p)| p.created).map(|(k, _)| k.clone())
        {
            plays.remove(&oldest);
        }
        plays.insert(play_session.clone(), play);
    }
    json_out(json!({"MediaSources": [source], "PlaySessionId": play_session}))
}

fn play_of(cx: &Cx, id: Option<String>) -> Option<(String, Play)> {
    let id = id?;
    let user = cx.ctx.user.as_ref()?;
    cx.jf().plays.lock().unwrap().get(&id).filter(|p| p.user_id == user.id).cloned().map(|p| (id, p))
}

/// The JSON of a FreeX handler's answer.
async fn body_json(r: Response) -> Value {
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap_or_default();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn object(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

// ====================================================================== files

/// `GET /Videos/{id}/stream`: the file as it is (direct play), with byte ranges.
pub async fn stream(cx: &Cx, raw: &str) -> Out {
    let u = cx.user()?;
    let fi = {
        let idx = cx.app.catalog.snapshot();
        let Some(Id::Item(id)) = parse_guid(raw) else { return Err(not_found()) };
        let it = visible_item(&idx, u, &id).map_err(|_| not_found())?;
        file_item(&idx, &it, cx.req.param("MediaSourceId"))
    };
    let f = fi.file.as_ref().filter(|_| fi.kind.playable()).ok_or_else(not_found)?;
    crate::files::send(std::path::Path::new(&f.path), 0, &cx.req.headers).await.map_err(super::err)
}

/// `GET /Videos/{id}/master.m3u8`: starts the conversion of the playback (once) and answers
/// with its playlist; the segments are under `hls/<grant>/` next to it.
pub async fn hls_playlist(cx: &Cx, raw: &str) -> Out {
    let u = cx.user()?.clone();
    let (play_id, mut play) = match play_of(cx, cx.req.param("PlaySessionId")) {
        Some(p) => p,
        None => {
            // A playlist URL without playback info: convert to what every player decodes.
            let idx = cx.app.catalog.snapshot();
            let Some(Id::Item(id)) = parse_guid(raw) else { return Err(not_found()) };
            let it = visible_item(&idx, &u, &id).map_err(|_| not_found())?;
            let fi = file_item(&idx, &it, cx.req.param("MediaSourceId"));
            let play = Play {
                user_id: u.id.clone(),
                item_id: fi.id.clone(),
                caps: json!({"mseVideo": ["h264"], "mseAudio": ["aac"], "directPlay": false}),
                quality: "auto".into(),
                audio: -1,
                subtitle: None,
                burn: false,
                start_ms: 0,
                direct: false,
                grant: None,
                session: None,
                created: now_ms(),
                client_id: cx.req.client.id(),
                device: cx.req.client.label(),
            };
            (new_id(), play)
        }
    };
    let alive = play.session.as_deref().is_some_and(|s| cx.app.transcoder.as_ref().is_some_and(|t| t.session(s).is_some()));
    if !alive || play.grant.is_none() {
        let mut caps = play.caps.clone();
        caps["directPlay"] = json!(false);
        let mut body = json!({
            "caps": caps,
            "quality": play.quality,
            "audio": play.audio,
            "startMs": play.start_ms,
            "tokenUrls": true,
            "client": play.client_id,
            "device": play.device,
        });
        if let Some(s) = &play.subtitle {
            body["subtitle"] = json!(s);
            body["burnSubtitle"] = json!(play.burn);
        }
        let r = crate::api::playback::start(
            State(cx.app.clone()),
            Extension(cx.ctx.clone()),
            Path(play.item_id.clone()),
            JsonBody(object(body)),
        )
        .await
        .map_err(super::err)?;
        let v = body_json(r).await;
        let url = v["url"].as_str().unwrap_or("");
        let Some(grant) =
            url.strip_prefix(crate::api::extras::CAST_PREFIX).and_then(|rest| rest.split('/').next()).filter(|g| !g.is_empty())
        else {
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into());
        };
        play.grant = Some(grant.to_string());
        play.session = v["session"].as_str().map(str::to_string);
        cx.jf().plays.lock().unwrap().insert(play_id, play.clone());
    }
    let grant = play.grant.clone().expect("set above");
    let r =
        crate::api::extras::cast_hls(State(cx.app.clone()), Path((grant.clone(), "index.m3u8".to_string()))).await.map_err(super::err)?;
    let bytes = axum::body::to_bytes(r.into_body(), 4 << 20).await.unwrap_or_default();
    let text = rewrite_playlist(&String::from_utf8_lossy(&bytes), &format!("hls/{grant}/"));
    Ok(([(header::CONTENT_TYPE, "application/vnd.apple.mpegurl"), (header::CACHE_CONTROL, "no-cache")], text).into_response())
}

/// Puts `prefix` in front of the file names of a playlist (segments and the init segment).
fn rewrite_playlist(text: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("#EXT-X-MAP:URI=\"") {
            out.push_str(&format!("#EXT-X-MAP:URI=\"{prefix}{rest}"));
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            out.push_str(prefix);
            out.push_str(line);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// `GET /Videos/{id}/hls/{grant}/{file}`: a file of a conversion; the grant in the path is the
/// access. Grants are case-sensitive, so they come from the path as sent.
pub async fn hls_file(cx: &Cx) -> Out {
    let (grant, file) = (cx.req.segs[3].clone(), cx.req.segs[4].clone());
    let r = crate::api::extras::cast_hls(State(cx.app.clone()), Path((grant, file))).await.map_err(super::err)?;
    Ok(r)
}

/// `GET /Videos/{id}/{source}/Subtitles/{index}/Stream.{format}`: WebVTT, or SubRip if asked for.
pub async fn subtitle(cx: &Cx, raw: &str, source: &str, index: &str, file: &str) -> Out {
    let u = cx.user()?;
    let index: i32 = index.parse().map_err(|_| not_found())?;
    let fi = {
        let idx = cx.app.catalog.snapshot();
        let Some(Id::Item(id)) = parse_guid(raw) else { return Err(not_found()) };
        let it = visible_item(&idx, u, &id).map_err(|_| not_found())?;
        file_item(&idx, &it, Some(source.to_string()))
    };
    let f = fi.file.as_ref().ok_or_else(not_found)?;
    let sid = subtitle_id(f, index).ok_or_else(not_found)?;
    let vtt = cx.app.subtitles.vtt(&fi, &sid).await.ok_or_else(not_found)?;
    let format = file.rsplit('.').next().unwrap_or("vtt");
    let (kind, text) = if matches!(format, "srt" | "subrip") {
        ("application/x-subrip; charset=utf-8", crate::media::subtitles::vtt_to_srt(&vtt))
    } else {
        ("text/vtt; charset=utf-8", vtt)
    };
    let mut r = text.into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=3600"));
    Ok(r)
}

// ====================================================================== position reports

/// The entry a report is about: `MediaSourceId` (a version) or `ItemId`, or the playback's.
fn report_item(cx: &Cx, play: Option<&Play>) -> Option<String> {
    let id = |name: &str| match cx.req.param(name).as_deref().and_then(parse_guid) {
        Some(Id::Item(i)) => Some(i),
        _ => None,
    };
    play.map(|p| p.item_id.clone()).or_else(|| id("MediaSourceId")).or_else(|| id("ItemId"))
}

/// The app and device a report counts for: those of its playback, else of the request.
fn client_of(cx: &Cx, play: Option<&Play>) -> (String, String) {
    match play {
        Some(p) => (p.client_id.clone(), p.device.clone()),
        None => (cx.req.client.id(), cx.req.client.label()),
    }
}

/// `POST /Sessions/Playing`: direct play shows up in the activity list now; a conversion did when
/// its playlist was loaded.
pub async fn started(cx: &Cx) -> Out {
    cx.user()?;
    let play = play_of(cx, cx.req.param("PlaySessionId"));
    if play.as_ref().is_some_and(|(_, p)| !p.direct) {
        return empty(StatusCode::NO_CONTENT);
    }
    let Some(item) = report_item(cx, play.as_ref().map(|(_, p)| p)) else { return empty(StatusCode::NO_CONTENT) };
    let (client, device) = client_of(cx, play.as_ref().map(|(_, p)| p));
    let body = json!({
        "player": "external",
        "client": client,
        "device": device,
        "startMs": cx.req.int("PositionTicks").map_or(0, ms),
        "audio": play.as_ref().map_or(-1, |(_, p)| p.audio),
    });
    // Errors (an entry that is gone) do not concern the player.
    let _ = crate::api::playback::start(State(cx.app.clone()), Extension(cx.ctx.clone()), Path(item), JsonBody(object(body))).await;
    empty(StatusCode::NO_CONTENT)
}

/// `POST /Sessions/Playing/Progress` and `/Stopped`: the position goes into the watch state.
pub async fn progress(cx: &Cx, stopped: bool) -> Out {
    let u = cx.user()?;
    let play = play_of(cx, cx.req.param("PlaySessionId"));
    let Some(item) = report_item(cx, play.as_ref().map(|(_, p)| p)) else { return empty(StatusCode::NO_CONTENT) };
    let (client, device) = client_of(cx, play.as_ref().map(|(_, p)| p));
    match cx.req.int("PositionTicks") {
        Some(t) => {
            let paused = cx.req.bool("IsPaused").unwrap_or(false);
            let body = json!({"offsetMs": ms(t).max(0), "client": client, "device": device,
                              "state": if paused { "paused" } else { "playing" }, "stopped": stopped});
            let _ =
                crate::api::playback::progress(State(cx.app.clone()), Extension(cx.ctx.clone()), Path(item), JsonBody(object(body))).await;
        }
        // Without a position the watch state stays as it is; only the activity ends.
        None if stopped => {
            cx.app.activity.progress(&u.id, &item, Some(&client), 0, "stopped", true);
        }
        None => {}
    }
    if stopped && let Some((id, p)) = play {
        if let (Some(s), Some(t)) = (&p.session, &cx.app.transcoder) {
            t.stop(s);
        }
        cx.jf().plays.lock().unwrap().remove(&id);
    }
    empty(StatusCode::NO_CONTENT)
}

/// `DELETE /Videos/ActiveEncodings`: the player gave up a conversion.
pub fn stop_encoding(cx: &Cx) -> Out {
    if let Some((_, p)) = play_of(cx, cx.req.param("PlaySessionId"))
        && let (Some(s), Some(t)) = (&p.session, &cx.app.transcoder)
    {
        t.stop(s);
    }
    empty(StatusCode::NO_CONTENT)
}

// ====================================================================== watch state

fn user_data_of(cx: &Cx, raw: &str) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let Some(Id::Item(id)) = parse_guid(raw) else { return Err(not_found()) };
    let it = visible_item(&idx, u, &id).map_err(|_| not_found())?;
    json_out(Dto::new(cx, &idx, u).user_data(&it))
}

pub fn user_data(cx: &Cx, raw: &str) -> Out {
    user_data_of(cx, raw)
}

/// Marks a movie or episode (or all episodes of a season or show) as watched or not.
pub async fn set_played(cx: &Cx, raw: &str, played: bool) -> Out {
    let Some(Id::Item(id)) = parse_guid(raw) else { return Err(not_found()) };
    let (app, ctx) = (State(cx.app.clone()), Extension(cx.ctx.clone()));
    let r = if played {
        crate::api::playback::watched(app, ctx, Path(id)).await
    } else {
        crate::api::playback::unwatched(app, ctx, Path(id)).await
    };
    r.map_err(super::err)?;
    user_data_of(cx, raw)
}

/// Favorites are the watchlist, which has movies and shows; other entries cannot be favorites.
pub fn set_favorite(cx: &Cx, raw: &str, on: bool) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let Some(Id::Item(id)) = parse_guid(raw) else { return Err(not_found()) };
    let it = visible_item(&idx, u, &id).map_err(|_| not_found())?;
    if matches!(it.kind, Kind::Movie | Kind::Show) {
        cx.app.db.set_watchlist(&u.id, &it.id, on);
    }
    drop(idx);
    user_data_of(cx, raw)
}

/// `POST /UserItems/{id}/UserData`: played flag, favorite, resume position.
pub async fn update_user_data(cx: &Cx, raw: &str) -> Out {
    if let Some(p) = cx.req.field("Played").and_then(Value::as_bool) {
        set_played(cx, raw, p).await?;
    }
    if let Some(f) = cx.req.field("IsFavorite").and_then(Value::as_bool) {
        set_favorite(cx, raw, f)?;
    }
    if let Some(t) = cx.req.field("PlaybackPositionTicks").and_then(Value::as_i64)
        && let Some(Id::Item(id)) = parse_guid(raw)
    {
        let body = json!({"offsetMs": ms(t).max(0)});
        let _ = crate::api::playback::progress(State(cx.app.clone()), Extension(cx.ctx.clone()), Path(id), JsonBody(object(body))).await;
    }
    user_data_of(cx, raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::AudioStream;

    fn file() -> MediaFile {
        MediaFile {
            container: "mkv".into(),
            format_name: "matroska,webm".into(),
            video_codec: "hevc".into(),
            pixel_format: "yuv420p10le".into(),
            width: 3840,
            height: 2160,
            bitrate: 30_000_000,
            audio_streams: vec![AudioStream { stream_index: 1, codec: "eac3".into(), channels: 6, ..Default::default() }],
            ..Default::default()
        }
    }

    #[test]
    fn device_profile() {
        let f = file();
        let tv = json!({
            "DirectPlayProfiles": [{"Type": "Video", "Container": "mp4,mkv", "VideoCodec": "h264,hevc", "AudioCodec": "aac,ac3,eac3"}],
            "TranscodingProfiles": [{"Type": "Video", "Protocol": "hls", "Container": "mp4", "VideoCodec": "h264,hevc", "AudioCodec": "aac,ac3"}],
        });
        let caps = capabilities(Some(&tv), &f, -1);
        assert_eq!(caps["directPlay"], true);
        assert_eq!(caps["video"], json!(["hevc"]));

        // An 8-bit-only HEVC decoder: no direct play, and the video is converted, not copied.
        let mut limited = tv.clone();
        limited["CodecProfiles"] = json!([{"Type": "Video", "Codec": "hevc",
            "Conditions": [{"Condition": "LessThanEqual", "Property": "VideoBitDepth", "Value": "8", "IsRequired": true}]}]);
        let caps = capabilities(Some(&limited), &f, -1);
        assert_eq!(caps["directPlay"], false);
        assert_eq!(caps["mseVideo"], json!(["h264"]));

        // Stereo only: E-AC3 5.1 is not copied.
        let mut stereo = tv.clone();
        stereo["CodecProfiles"] = json!([{"Type": "VideoAudio",
            "Conditions": [{"Condition": "LessThanEqual", "Property": "AudioChannels", "Value": "2"}]}]);
        let caps = capabilities(Some(&stereo), &f, -1);
        assert_eq!(caps["directPlay"], false);
        assert!(!caps["mseAudio"].as_array().unwrap().contains(&json!("eac3")));

        // No profile: the app plays the file itself.
        assert_eq!(capabilities(None, &f, -1)["directPlay"], true);
    }

    #[test]
    fn quality_from_bitrate() {
        let f = file();
        assert_eq!(quality_for(None, &f), "auto");
        assert_eq!(quality_for(Some(120_000_000), &f), "original");
        assert_eq!(quality_for(Some(10_000_000), &f), "8000");
        assert_eq!(quality_for(Some(100_000), &f), "1000");
    }

    #[test]
    fn playlist_paths() {
        let p = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6.0,\ns0.m4s\n#EXT-X-ENDLIST\n";
        let r = rewrite_playlist(p, "hls/g/");
        assert!(r.contains("#EXT-X-MAP:URI=\"hls/g/init.mp4\""));
        assert!(r.contains("\nhls/g/s0.m4s\n"));
        assert!(r.contains("#EXTINF:6.0,\n"));
    }
}
