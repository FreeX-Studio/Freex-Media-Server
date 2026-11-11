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

//! FreeX entries as Jellyfin objects (`BaseItemDto`, `UserItemDataDto`, `MediaSourceInfo`,
//! `MediaStream`). Field names are PascalCase; every field the apps' models require is filled.

use super::{Cx, Jellyfin, genre_guid, item_guid, library_guid};
use crate::api::media::{View, art_version};
use crate::db::{Library, User};
use crate::media::catalog::Index;
use crate::media::{Item, Kind, MediaFile};
use serde_json::{Value, json};
use std::sync::Arc;

/// Jellyfin counts time in ticks of 100 ns.
pub fn ticks(ms: i64) -> i64 {
    ms.saturating_mul(10_000)
}

pub fn ms(ticks: i64) -> i64 {
    ticks / 10_000
}

/// A time in ms as the apps parse it.
pub fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms.max(0))
        .map(|d| format!("{}.{:07}Z", d.format("%Y-%m-%dT%H:%M:%S"), d.timestamp_subsec_nanos() / 100))
        .unwrap_or_else(|| "0001-01-01T00:00:00.0000000Z".into())
}

/// The signed tag of an image: the apps send it back with the image request, which therefore
/// needs no sign-in (they load images without one). Changes with the artwork.
pub fn image_tag(jf: &Jellyfin, idx: &Index, it: &Item, kind: &str) -> String {
    jf.sign(&format!("{}|{kind}|{}", it.id, art_version(idx, it)))
}

/// Jellyfin's name of the kind of an entry.
pub fn type_name(it: &Item) -> &'static str {
    match it.kind {
        Kind::Movie => "Movie",
        Kind::Show => "Series",
        Kind::Season => "Season",
        Kind::Episode => "Episode",
        Kind::Extra => "Video",
    }
}

/// The FreeX kind for a Jellyfin type name (lower case); None for types FreeX does not have.
pub fn kind_of(type_name: &str) -> Option<Kind> {
    match type_name {
        "movie" => Some(Kind::Movie),
        "series" => Some(Kind::Show),
        "season" => Some(Kind::Season),
        "episode" => Some(Kind::Episode),
        "video" | "trailer" => Some(Kind::Extra),
        _ => None,
    }
}

fn extra_type(t: &str) -> &'static str {
    match t {
        "trailer" => "Trailer",
        "featurette" => "Featurette",
        "behindTheScenes" => "BehindTheScenes",
        "deletedScene" => "DeletedScene",
        "interview" => "Interview",
        "scene" => "Scene",
        "short" => "Short",
        _ => "Clip",
    }
}

pub fn collection_type(l: &Library) -> &'static str {
    if l.kind == "shows" { "tvshows" } else { "movies" }
}

/// Builds the objects for one user.
pub struct Dto<'a> {
    pub cx: &'a Cx,
    pub idx: &'a Index,
    pub view: View<'a>,
}

impl<'a> Dto<'a> {
    pub fn new(cx: &'a Cx, idx: &'a Index, user: &'a User) -> Dto<'a> {
        Dto { cx, idx, view: View::new(&cx.app, idx, user) }
    }

    fn server_id(&self) -> &str {
        &self.cx.jf().server_id
    }

    fn tag(&self, it: &Item, kind: &str) -> String {
        image_tag(self.cx.jf(), self.idx, it, kind)
    }

    pub fn library(&self, l: &Library) -> Value {
        let id = library_guid(&l.id);
        let count = self.idx.top_level(&l.id).iter().filter(|it| self.view.sees(it)).count();
        json!({
            "Name": l.name,
            "ServerId": self.server_id(),
            "Id": id,
            "Etag": id,
            "DateCreated": iso(l.created_at),
            "CanDelete": false,
            "CanDownload": false,
            "SortName": l.name.to_lowercase(),
            "ExternalUrls": [],
            "EnableMediaSourceDisplay": true,
            "Taglines": [],
            "Genres": [],
            "PlayAccess": "Full",
            "RemoteTrailers": [],
            "ProviderIds": {},
            "IsFolder": true,
            "Type": "CollectionFolder",
            "CollectionType": collection_type(l),
            "People": [],
            "Studios": [],
            "GenreItems": [],
            "LocalTrailerCount": 0,
            "UserData": {"PlaybackPositionTicks": 0, "PlayCount": 0, "IsFavorite": false, "Played": false, "Key": id, "ItemId": id},
            "ChildCount": count,
            "SpecialFeatureCount": 0,
            "DisplayPreferencesId": id,
            "Tags": [],
            "ImageTags": {},
            "BackdropImageTags": [],
            "ImageBlurHashes": {},
            "LocationType": "FileSystem",
            "MediaType": "Unknown",
            "LockedFields": [],
            "LockData": false,
        })
    }

    pub fn user_data(&self, it: &Item) -> Value {
        let id = item_guid(&it.id);
        let favorite = matches!(it.kind, Kind::Movie | Kind::Show) && self.view.on_watchlist(&it.id);
        let mut m = json!({"Key": id, "ItemId": id, "IsFavorite": favorite});
        if it.kind.playable() {
            let p = self.view.progress(it.key());
            m["PlaybackPositionTicks"] = json!(ticks(p.offset_ms));
            m["PlayCount"] = json!(p.view_count);
            m["Played"] = json!(p.watched);
            let duration = it.duration_ms();
            if p.offset_ms > 0 && duration > 0 {
                m["PlayedPercentage"] = json!((p.offset_ms as f64 / duration as f64 * 100.0).min(100.0));
            }
            if p.updated_at > 0 {
                m["LastPlayedDate"] = json!(iso(p.updated_at));
            }
        } else {
            let (leaves, viewed) = self.view.count_leaves(it);
            m["PlaybackPositionTicks"] = json!(0);
            m["PlayCount"] = json!(0);
            m["Played"] = json!(leaves > 0 && leaves == viewed);
            m["UnplayedItemCount"] = json!(leaves - viewed);
            if leaves > 0 && viewed > 0 {
                m["PlayedPercentage"] = json!(viewed as f64 / leaves as f64 * 100.0);
            }
        }
        m
    }

    /// An entry; `full` adds media sources, streams and chapters (the item page).
    pub fn item(&self, it: &Arc<Item>, full: bool) -> Value {
        let id = item_guid(&it.id);
        let genres: Vec<&str> = it.genre_list().collect();
        let playable = it.kind.playable();
        let mut m = json!({
            "Name": self.view.title_of(it),
            "OriginalTitle": if it.original_title.is_empty() { Value::Null } else { json!(it.original_title) },
            "ServerId": self.server_id(),
            "Id": id,
            "Etag": art_version(self.idx, it),
            "DateCreated": iso(it.added_at),
            "CanDelete": false,
            "CanDownload": playable,
            "SortName": it.sort_title,
            "Overview": it.summary,
            "Taglines": if it.tagline.is_empty() { json!([]) } else { json!([it.tagline]) },
            "Genres": genres,
            "GenreItems": genres.iter().map(|g| json!({"Name": g, "Id": genre_guid(g)})).collect::<Vec<_>>(),
            "IsFolder": !playable,
            "Type": type_name(it),
            "MediaType": if playable { "Video" } else { "Unknown" },
            "LocationType": "FileSystem",
            "PlayAccess": "Full",
            "ExternalUrls": [],
            "People": [],
            "Studios": [],
            "Tags": [],
            "RemoteTrailers": [],
            "LockedFields": [],
            "LockData": false,
            "EnableMediaSourceDisplay": true,
            "ImageBlurHashes": {},
            "ProviderIds": self.provider_ids(it),
            "UserData": self.user_data(it),
            "PrimaryImageAspectRatio": if matches!(it.kind, Kind::Episode | Kind::Extra) { 16.0 / 9.0 } else { 2.0 / 3.0 },
        });
        if it.rating > 0.0 {
            m["CommunityRating"] = json!((it.rating * 10.0).round() / 10.0);
        }
        if !it.content_rating.is_empty() {
            m["OfficialRating"] = json!(it.content_rating);
        }
        if it.year > 0 {
            m["ProductionYear"] = json!(it.year);
            m["PremiereDate"] = json!(format!("{:04}-01-01T00:00:00.0000000Z", it.year));
        }

        // Artwork: the entry's own, plus the show's for seasons and episodes.
        let mut tags = json!({"Primary": self.tag(it, "primary")});
        if matches!(it.kind, Kind::Movie | Kind::Show) {
            tags["Thumb"] = json!(self.tag(it, "thumb"));
            m["BackdropImageTags"] = json!([self.tag(it, "backdrop")]);
        } else {
            m["BackdropImageTags"] = json!([]);
        }
        m["ImageTags"] = tags;

        let show = self.idx.show_of(it);
        let library = || library_guid(&it.library_id);
        match it.kind {
            Kind::Movie | Kind::Show => {
                m["ParentId"] = json!(library());
                let extras = self.idx.extras(&it.id);
                let trailers = extras.iter().filter(|e| e.extra_type == "trailer").count();
                m["LocalTrailerCount"] = json!(trailers);
                m["SpecialFeatureCount"] = json!(extras.len() - trailers);
            }
            Kind::Season | Kind::Episode => {
                if let Some(parent) = self.idx.parent(it) {
                    m["ParentId"] = json!(item_guid(&parent.id));
                }
            }
            Kind::Extra => {
                m["ExtraType"] = json!(extra_type(&it.extra_type));
                if let Some(owner) = self.idx.parent(it) {
                    m["ParentId"] = json!(item_guid(&owner.id));
                }
            }
        }
        if let Some(show) = show {
            let sid = item_guid(&show.id);
            m["SeriesName"] = json!(show.title);
            m["SeriesId"] = json!(sid);
            m["SeriesPrimaryImageTag"] = json!(self.tag(show, "primary"));
            m["ParentBackdropItemId"] = json!(sid);
            m["ParentBackdropImageTags"] = json!([self.tag(show, "backdrop")]);
            m["ParentThumbItemId"] = json!(sid);
            m["ParentThumbImageTag"] = json!(self.tag(show, "thumb"));
        }
        if matches!(it.kind, Kind::Season | Kind::Episode) {
            m["IndexNumber"] = json!(it.index);
        }
        if it.kind == Kind::Episode
            && let Some(season) = self.idx.parent(it)
        {
            m["ParentIndexNumber"] = json!(season.index);
            m["SeasonId"] = json!(item_guid(&season.id));
            m["SeasonName"] = json!(self.view.title_of(season));
        }
        if matches!(it.kind, Kind::Show | Kind::Season) {
            let (leaves, _) = self.view.count_leaves(it);
            m["ChildCount"] = json!(self.idx.children(&it.id).len());
            m["RecursiveItemCount"] = json!(leaves);
        }
        if playable && let Some(f) = &it.file {
            m["RunTimeTicks"] = json!(ticks(f.duration_ms));
            m["Container"] = json!(f.container);
            m["VideoType"] = json!("VideoFile");
            m["IsHD"] = json!(f.height >= 720);
            if f.width > 0 {
                m["Width"] = json!(f.width);
                m["Height"] = json!(f.height);
            }
            m["HasSubtitles"] = json!(!f.subtitles.is_empty() || !f.subtitle_streams.is_empty());
            let owner = self.idx.item(it.key()).cloned().unwrap_or_else(|| it.clone());
            m["MediaSourceCount"] = json!(owner.versions.len().max(1));
            if full {
                m["MediaSources"] = json!(self.media_sources(it, None));
                m["MediaStreams"] = json!(streams(&id, &id, f, None));
                m["Chapters"] = json!(
                    f.chapters
                        .iter()
                        .enumerate()
                        .map(|(i, c)| json!({
                            "StartPositionTicks": ticks(c.start_ms),
                            "Name": if c.title.is_empty() { format!("Chapter {}", i + 1) } else { c.title.clone() },
                            "ImageDateModified": "0001-01-01T00:00:00.0000000Z",
                        }))
                        .collect::<Vec<_>>()
                );
            }
        }
        m
    }

    fn provider_ids(&self, it: &Item) -> Value {
        let mut p = json!({});
        if it.tmdb_hint > 0 {
            p["Tmdb"] = json!(it.tmdb_hint.to_string());
        }
        if !it.imdb_hint.is_empty() {
            p["Imdb"] = json!(it.imdb_hint);
        }
        if it.tvdb_hint > 0 {
            p["Tvdb"] = json!(it.tvdb_hint.to_string());
        }
        p
    }

    /// One media source per file of the entry (movies with several versions have several).
    /// `token` adds subtitle URLs the player can load without the sign-in header.
    pub fn media_sources(&self, it: &Arc<Item>, token: Option<&str>) -> Vec<Value> {
        let owner = self.idx.item(it.key()).cloned().unwrap_or_else(|| it.clone());
        let versions = self.idx.versions_of(&owner);
        let files: Vec<&Arc<Item>> = if versions.is_empty() { vec![it] } else { versions };
        files
            .iter()
            .filter_map(|v| {
                let label = owner.versions.iter().find(|x| x.id == v.id).map(|x| x.label.clone()).unwrap_or_default();
                media_source(&item_guid(&it.id), v, &label, token)
            })
            .collect()
    }
}

/// A file as `MediaSourceInfo`; direct play allowed, decided later by the playback info.
pub fn media_source(item_id: &str, file_item: &Item, label: &str, token: Option<&str>) -> Option<Value> {
    let f = file_item.file.as_ref()?;
    let msid = item_guid(&file_item.id);
    let name = if !label.is_empty() {
        label.to_string()
    } else {
        let r = f.resolution_label();
        if r.is_empty() { file_item.title.clone() } else { format!("{} - {r}", file_item.title) }
    };
    let streams = streams(item_id, &msid, f, token);
    let default_audio = f.audio_streams.iter().find(|a| a.is_default).or(f.audio_streams.first()).map(|a| a.stream_index);
    Some(json!({
        "Protocol": "File",
        "Id": msid,
        "Path": f.file_name(),
        "Type": "Default",
        "Container": f.container,
        "Size": f.size,
        "Name": name,
        "IsRemote": false,
        "ETag": format!("{:x}", f.modified),
        "RunTimeTicks": ticks(f.duration_ms),
        "ReadAtNativeFramerate": false,
        "IgnoreDts": false,
        "IgnoreIndex": false,
        "GenPtsInput": false,
        "SupportsTranscoding": true,
        "SupportsDirectStream": false,
        "SupportsDirectPlay": true,
        "IsInfiniteStream": false,
        "UseMostCompatibleTranscodingProfile": false,
        "RequiresOpening": false,
        "RequiresClosing": false,
        "RequiresLooping": false,
        "SupportsProbing": true,
        "VideoType": "VideoFile",
        "MediaStreams": streams,
        "MediaAttachments": [],
        "Formats": [],
        "Bitrate": if f.bitrate > 0 { json!(f.bitrate) } else { Value::Null },
        "RequiredHttpHeaders": {},
        "TranscodingSubProtocol": "http",
        "DefaultAudioStreamIndex": default_audio,
        "HasSegments": false,
    }))
}

/// Index of the first external subtitle in the Jellyfin numbering: after all streams of the file.
fn external_base(f: &MediaFile) -> i32 {
    let max = f
        .audio_streams
        .iter()
        .map(|a| a.stream_index)
        .chain(f.subtitle_streams.iter().map(|s| s.stream_index))
        .chain(std::iter::once(f.video_stream_index))
        .max()
        .unwrap_or(0);
    max.max(0) + 1
}

/// The FreeX subtitle id (`e3`, `x0`) of a Jellyfin stream index.
pub fn subtitle_id(f: &MediaFile, index: i32) -> Option<String> {
    if f.subtitle_streams.iter().any(|s| s.stream_index == index) {
        return Some(format!("e{index}"));
    }
    let i = index - external_base(f);
    (i >= 0 && (i as usize) < f.subtitles.len()).then(|| format!("x{i}"))
}

/// Whether a Jellyfin subtitle index names a text subtitle (sent as WebVTT, never burned in).
pub fn subtitle_is_text(f: &MediaFile, index: i32) -> bool {
    match f.subtitle_streams.iter().find(|s| s.stream_index == index) {
        Some(s) => s.text_based(),
        None => subtitle_id(f, index).is_some(),
    }
}

/// The position among the audio streams of a Jellyfin stream index; -1 if unknown.
pub fn audio_position(f: &MediaFile, index: i32) -> i32 {
    f.audio_streams.iter().position(|a| a.stream_index == index).map_or(-1, |p| p as i32)
}

/// Jellyfin's subtitle format names.
pub fn subtitle_codec(codec: &str) -> String {
    match codec.to_lowercase().as_str() {
        "hdmv_pgs_subtitle" | "pgs" => "pgssub".into(),
        "dvd_subtitle" => "dvdsub".into(),
        "dvb_subtitle" => "dvbsub".into(),
        "srt" => "subrip".into(),
        "vtt" => "webvtt".into(),
        c => c.to_string(),
    }
}

/// Jellyfin's `VideoRangeType` of a file; Dolby Vision says what its base layer is.
pub fn range_type(f: &MediaFile) -> &'static str {
    match (f.dovi_profile, f.color_transfer.as_str()) {
        (5, _) => "DOVI",
        (7, _) => "DOVIWithEL",
        (p, "smpte2084") if p > 0 => "DOVIWithHDR10",
        (p, "arib-std-b67") if p > 0 => "DOVIWithHLG",
        (p, _) if p > 0 => "DOVIWithSDR",
        (_, "smpte2084") => "HDR10",
        (_, "arib-std-b67") => "HLG",
        _ => "SDR",
    }
}

fn layout(channels: i32) -> &'static str {
    match channels {
        1 => "mono",
        2 => "stereo",
        6 => "5.1",
        8 => "7.1",
        _ => "",
    }
}

fn language_label(lang: &str) -> String {
    if lang.is_empty() || lang == "und" { "Unknown".into() } else { lang.to_uppercase() }
}

/// The streams of a file. With a `token`, text subtitles carry a URL to load them as WebVTT.
pub fn streams(item_id: &str, msid: &str, f: &MediaFile, token: Option<&str>) -> Vec<Value> {
    let mut out = Vec::new();
    if !f.video_codec.is_empty() {
        let hdr = f.hdr();
        let range_type = range_type(f);
        let bits = if f.pixel_format.contains("12") {
            12
        } else if f.pixel_format.contains("10") {
            10
        } else {
            8
        };
        let codec = crate::media::playback::codec_kind(&f.video_codec);
        let res = f.resolution_label();
        out.push(json!({
            "Type": "Video",
            "Index": f.video_stream_index.max(0),
            "Codec": codec,
            "Profile": if f.video_profile.is_empty() { Value::Null } else { json!(f.video_profile) },
            "Width": f.width,
            "Height": f.height,
            "BitRate": if f.bitrate > 0 { json!(f.bitrate) } else { Value::Null },
            "BitDepth": bits,
            "PixelFormat": f.pixel_format,
            "VideoRange": if hdr.is_some() { "HDR" } else { "SDR" },
            "VideoRangeType": range_type,
            "IsAVC": codec == "h264",
            "IsInterlaced": false,
            "IsDefault": true,
            "IsForced": false,
            "IsHearingImpaired": false,
            "IsOriginal": false,
            "IsExternal": false,
            "IsTextSubtitleStream": false,
            "SupportsExternalStream": false,
            "IsAnamorphic": false,
            "AudioSpatialFormat": "None",
            "DisplayTitle": format!("{res} {} {range_type}", codec.to_uppercase()).trim().to_string(),
        }));
    }
    for a in &f.audio_streams {
        let codec = crate::media::playback::codec_kind(&a.codec);
        let mut title = format!("{} - {} {}", language_label(&a.language), codec.to_uppercase(), layout(a.channels)).trim().to_string();
        if a.is_default {
            title.push_str(" - Default");
        }
        out.push(json!({
            "Type": "Audio",
            "Index": a.stream_index,
            "Codec": codec,
            "Language": if a.language.is_empty() { Value::Null } else { json!(a.language) },
            "Title": if a.title.is_empty() { Value::Null } else { json!(a.title) },
            "Channels": a.channels,
            "ChannelLayout": layout(a.channels),
            "BitRate": if a.bitrate > 0 { json!(a.bitrate) } else { Value::Null },
            "IsInterlaced": false,
            "IsDefault": a.is_default,
            "IsForced": false,
            "IsHearingImpaired": false,
            "IsOriginal": false,
            "IsExternal": false,
            "IsTextSubtitleStream": false,
            "SupportsExternalStream": false,
            "VideoRange": "Unknown",
            "VideoRangeType": "Unknown",
            "AudioSpatialFormat": "None",
            "DisplayTitle": title,
        }));
    }
    let subtitle = |index: i32, codec: &str, lang: &str, title: &str, forced: bool, default: bool, external: bool, text: bool| {
        let mut display = format!("{} - {}", language_label(lang), subtitle_codec(codec).to_uppercase());
        if forced {
            display.push_str(" - Forced");
        }
        let mut s = json!({
            "Type": "Subtitle",
            "Index": index,
            "Codec": subtitle_codec(codec),
            "Language": if lang.is_empty() { Value::Null } else { json!(lang) },
            "Title": if title.is_empty() { Value::Null } else { json!(title) },
            "IsInterlaced": false,
            "IsDefault": default,
            "IsForced": forced,
            "IsHearingImpaired": false,
            "IsOriginal": false,
            "IsExternal": external,
            "IsTextSubtitleStream": text,
            "SupportsExternalStream": text,
            "VideoRange": "Unknown",
            "VideoRangeType": "Unknown",
            "AudioSpatialFormat": "None",
            "DisplayTitle": display,
        });
        if text && let Some(t) = token {
            s["DeliveryMethod"] = json!("External");
            s["DeliveryUrl"] = json!(format!("/Videos/{item_id}/{msid}/Subtitles/{index}/0/Stream.vtt?ApiKey={t}"));
            s["IsExternalUrl"] = json!(false);
        } else if token.is_some() {
            s["DeliveryMethod"] = json!(if external { "External" } else { "Embed" });
        }
        s
    };
    for s in &f.subtitle_streams {
        out.push(subtitle(s.stream_index, &s.codec, &s.language, &s.title, s.forced, s.is_default, false, s.text_based()));
    }
    let base = external_base(f);
    for (i, s) in f.subtitles.iter().enumerate() {
        out.push(subtitle(base + i as i32, &s.format, &s.language, "", s.forced, false, true, true));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{AudioStream, Subtitle, SubtitleStream};

    fn file() -> MediaFile {
        MediaFile {
            video_codec: "h264".into(),
            video_stream_index: 0,
            audio_streams: vec![
                AudioStream { stream_index: 1, codec: "ac3".into(), channels: 6, language: "ger".into(), ..Default::default() },
                AudioStream { stream_index: 2, codec: "aac".into(), channels: 2, language: "eng".into(), ..Default::default() },
            ],
            subtitle_streams: vec![
                SubtitleStream { stream_index: 3, codec: "subrip".into(), language: "ger".into(), ..Default::default() },
                SubtitleStream { stream_index: 4, codec: "hdmv_pgs_subtitle".into(), language: "eng".into(), ..Default::default() },
            ],
            subtitles: vec![Subtitle { path: "/m/a.en.srt".into(), language: "en".into(), format: "srt".into(), forced: false }],
            ..Default::default()
        }
    }

    #[test]
    fn stream_numbering() {
        let f = file();
        assert_eq!(audio_position(&f, 2), 1);
        assert_eq!(audio_position(&f, 9), -1);
        assert_eq!(subtitle_id(&f, 3).as_deref(), Some("e3"));
        assert_eq!(subtitle_id(&f, 5).as_deref(), Some("x0"), "external subtitles follow the streams of the file");
        assert_eq!(subtitle_id(&f, 6), None);
        assert!(subtitle_is_text(&f, 3) && subtitle_is_text(&f, 5) && !subtitle_is_text(&f, 4));
        let s = streams("i", "m", &f, Some("tok"));
        assert_eq!(s.len(), 6);
        assert_eq!(s[5]["Index"], 5);
        assert_eq!(s[3]["DeliveryUrl"], "/Videos/i/m/Subtitles/3/0/Stream.vtt?ApiKey=tok");
        assert_eq!(s[4]["DeliveryMethod"], "Embed");
        assert_eq!(s[4]["Codec"], "pgssub");
    }

    #[test]
    fn times() {
        assert_eq!(ticks(1500), 15_000_000);
        assert_eq!(ms(15_000_000), 1500);
        assert_eq!(iso(0), "1970-01-01T00:00:00.0000000Z");
        assert_eq!(iso(1_700_000_000_123), "2023-11-14T22:13:20.1230000Z");
    }
}
