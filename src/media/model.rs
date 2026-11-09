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

//! Library entries (movies, shows, seasons, episodes) and the technical data of their files.
//!
//! The structs are stored as JSON in the database (all fields, camelCase); `to_public` gives the
//! API view, which never contains file system paths.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Version of the probe data. Files probed with an older version are probed again on the next
/// scan, so new fields get filled for existing libraries.
pub const PROBE_VERSION: i32 = 3;

/// Subtitle codecs that can be converted to WebVTT; everything else is a bitmap format.
const TEXT_SUBTITLE_CODECS: [&str; 16] = [
    "subrip",
    "srt",
    "ass",
    "ssa",
    "webvtt",
    "mov_text",
    "text",
    "ttml",
    "microdvd",
    "subviewer",
    "subviewer1",
    "realtext",
    "sami",
    "stl",
    "mpl2",
    "vplayer",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Movie,
    Show,
    Season,
    Episode,
}

impl Kind {
    /// Movies and episodes have a file; shows and seasons only group them.
    pub fn playable(self) -> bool {
        matches!(self, Kind::Movie | Kind::Episode)
    }
}

/// One entry of a library.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Item {
    pub id: String,
    pub library_id: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    pub parent_id: Option<String>,
    pub title: String,
    pub sort_title: String,
    pub original_title: String,
    pub year: i32,
    /// Season number for seasons, episode number for episodes.
    pub index: i32,
    pub summary: String,
    pub tagline: String,
    /// Comma separated.
    pub genres: String,
    pub rating: f64,
    pub added_at: i64,
    /// Folder of a movie, show or season; used for artwork lookups.
    pub folder: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub file: Option<MediaFile>,
    /// True if a `.nfo` file supplied the metadata; online metadata then only fills gaps.
    pub has_nfo: bool,
    /// TMDB id from the `.nfo` or the name (`{tmdb-603}`), 0 if unknown.
    pub tmdb_hint: i64,
    /// IMDb id (`tt0133093`) from the `.nfo` or the name, empty if unknown.
    pub imdb_hint: String,
    /// TVDB id of a show from the `.nfo` or the name, 0 if unknown.
    pub tvdb_hint: i64,
}

impl Item {
    pub fn new(id: String, library_id: &str, kind: Kind) -> Item {
        Item { id, library_id: library_id.to_string(), kind, ..Item::default() }
    }

    /// API view without file system paths; watch state is added by the caller.
    pub fn to_public(&self) -> Value {
        let mut m = json!({
            "id": self.id,
            "libraryId": self.library_id,
            "type": self.kind,
            "parentId": self.parent_id,
            "title": self.title,
            "originalTitle": self.original_title,
            "year": self.year,
            "index": self.index,
            "summary": self.summary,
            "tagline": self.tagline,
            "genres": self.genres,
            "rating": self.rating,
            "addedAt": self.added_at,
            "hasPoster": self.poster_path.is_some(),
            "hasBackdrop": self.backdrop_path.is_some(),
        });
        if let Some(f) = &self.file {
            m["media"] = f.to_public();
        }
        m
    }

    pub fn genre_list(&self) -> impl Iterator<Item = &str> {
        self.genres.split(',').map(str::trim).filter(|g| !g.is_empty())
    }

    pub fn has_genre(&self, genre: &str) -> bool {
        let genre = genre.trim().to_lowercase();
        self.genre_list().any(|g| g.to_lowercase() == genre)
    }

    pub fn duration_ms(&self) -> i64 {
        self.file.as_ref().map_or(0, |f| f.duration_ms)
    }
}

/// A subtitle file lying next to the video (`Movie.de.srt`, `Movie.en.forced.vtt`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Subtitle {
    pub path: String,
    pub language: String,
    pub format: String,
    pub forced: bool,
}

/// An audio stream inside the file; `stream_index` is the absolute index for ffmpeg (`-map 0:<n>`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioStream {
    pub stream_index: i32,
    pub codec: String,
    pub channels: i32,
    pub language: String,
    pub title: String,
    #[serde(rename = "default")]
    pub is_default: bool,
    pub bitrate: i64,
}

/// A subtitle stream inside the file. Bitmap subtitles (PGS, VobSub) can only be burned in.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubtitleStream {
    pub stream_index: i32,
    pub codec: String,
    pub language: String,
    pub title: String,
    pub forced: bool,
    #[serde(rename = "default")]
    pub is_default: bool,
}

impl SubtitleStream {
    pub fn text_based(&self) -> bool {
        TEXT_SUBTITLE_CODECS.contains(&self.codec.as_str())
    }
}

/// A chapter of the file, as the container lists it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Chapter {
    pub start_ms: i64,
    pub end_ms: i64,
    /// Empty when the file has no name for it.
    pub title: String,
}

/// The video file of a movie or episode.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MediaFile {
    pub path: String,
    pub size: i64,
    /// Modification time in ms.
    pub modified: i64,
    /// File extension, lower case.
    pub container: String,
    /// Container names as ffprobe reports them, e.g. "matroska,webm".
    pub format_name: String,
    pub duration_ms: i64,
    pub video_codec: String,
    pub video_profile: String,
    pub pixel_format: String,
    pub video_stream_index: i32,
    pub audio_codec: String,
    pub width: i32,
    pub height: i32,
    pub bitrate: i64,
    pub audio_channels: i32,
    pub probed: bool,
    pub probe_version: i32,
    pub subtitles: Vec<Subtitle>,
    pub audio_streams: Vec<AudioStream>,
    pub subtitle_streams: Vec<SubtitleStream>,
    pub chapters: Vec<Chapter>,
}

impl Default for MediaFile {
    fn default() -> Self {
        MediaFile {
            path: String::new(),
            size: 0,
            modified: 0,
            container: String::new(),
            format_name: String::new(),
            duration_ms: 0,
            video_codec: String::new(),
            video_profile: String::new(),
            pixel_format: String::new(),
            video_stream_index: -1,
            audio_codec: String::new(),
            width: 0,
            height: 0,
            bitrate: 0,
            audio_channels: 0,
            probed: false,
            probe_version: 0,
            subtitles: Vec::new(),
            audio_streams: Vec::new(),
            subtitle_streams: Vec::new(),
            chapters: Vec::new(),
        }
    }
}

impl MediaFile {
    pub fn probe_current(&self) -> bool {
        self.probed && self.probe_version >= PROBE_VERSION
    }

    pub fn file_name(&self) -> &str {
        self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path)
    }

    /// Short label like "4K", "1080p" or "SD", from the frame size.
    pub fn resolution_label(&self) -> &'static str {
        let (w, h) = (self.width, self.height);
        if w <= 0 && h <= 0 {
            ""
        } else if w >= 3200 || h >= 2000 {
            "4K"
        } else if w >= 1800 || h >= 1000 {
            "1080p"
        } else if w >= 1200 || h >= 700 {
            "720p"
        } else if h >= 560 {
            "576p"
        } else {
            "SD"
        }
    }

    /// API view. Subtitles get an id: `x<n>` for files next to the video, `e<streamIndex>` for
    /// streams inside it.
    pub fn to_public(&self) -> Value {
        let audio: Vec<Value> = self
            .audio_streams
            .iter()
            .enumerate()
            .map(|(i, a)| {
                json!({"index": i, "codec": a.codec, "channels": a.channels, "language": a.language,
                       "title": a.title, "default": a.is_default})
            })
            .collect();
        let mut subs: Vec<Value> = self
            .subtitles
            .iter()
            .enumerate()
            .map(|(i, s)| {
                json!({"id": format!("x{i}"), "language": s.language, "title": "", "forced": s.forced, "default": false,
                       "codec": s.format, "textBased": true, "external": true})
            })
            .collect();
        subs.extend(self.subtitle_streams.iter().map(|s| {
            json!({"id": format!("e{}", s.stream_index), "language": s.language, "title": s.title, "forced": s.forced,
                   "default": s.is_default, "codec": s.codec, "textBased": s.text_based(), "external": false})
        }));
        json!({
            "size": self.size,
            "container": self.container,
            "durationMs": self.duration_ms,
            "videoCodec": self.video_codec,
            "videoProfile": self.video_profile,
            "pixelFormat": self.pixel_format,
            "audioCodec": self.audio_codec,
            "width": self.width,
            "height": self.height,
            "bitrate": self.bitrate,
            "audioChannels": self.audio_channels,
            "resolution": self.resolution_label(),
            "fileName": self.file_name(),
            "audio": audio,
            "subtitles": subs,
            "chapters": self.chapters.iter().map(|c| json!({"startMs": c.start_ms, "endMs": c.end_ms, "title": c.title})).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_round_trip() {
        let mut it = Item::new("abc".into(), "lib", Kind::Episode);
        it.parent_id = Some("season".into());
        it.file = Some(MediaFile { path: "/m/a.mkv".into(), width: 1920, height: 1080, ..MediaFile::default() });
        let back: Item = serde_json::from_str(&serde_json::to_string(&it).unwrap()).unwrap();
        assert_eq!(back.kind, Kind::Episode);
        assert_eq!(back.file.as_ref().unwrap().video_stream_index, -1);
        let p = back.to_public();
        assert_eq!(p["type"], "episode");
        assert_eq!(p["media"]["resolution"], "1080p");
        assert_eq!(p["media"]["fileName"], "a.mkv");
        assert!(p.get("posterPath").is_none());
    }
}
