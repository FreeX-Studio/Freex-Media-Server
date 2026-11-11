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

//! The folders the players browse, as DIDL-Lite:
//!
//! ```text
//! 0                     root: "Continue watching", "Recently added", the libraries
//! ├─ resume             started movies and episodes, last played first
//! ├─ recent             newest movies and episodes
//! └─ lib:<id>           a library: its movies, or its shows
//!    └─ item:<show>     a show: its seasons
//!       └─ item:<season>  a season: its episodes
//! ```
//!
//! Movies and episodes are `item:<id>`; everything is seen with the libraries and age limit of
//! the DLNA user.

use super::xml::esc;
use crate::AppState;
use crate::api::media::View;
use crate::db::{Library, User};
use crate::media::catalog::Index;
use crate::media::{Item, Kind, MediaFile};
use std::sync::Arc;

/// Entries in "Continue watching" and "Recently added".
const ROW: usize = 50;

/// DLNA flags for streamed files: byte ranges (OP=01), the original file (CI=0), streaming and
/// background transfer, DLNA 1.5.
pub const FLAGS: &str = "DLNA.ORG_OP=01;DLNA.ORG_CI=0;DLNA.ORG_FLAGS=01700000000000000000000000000000";

/// The MIME type players expect for a container.
pub fn mime(f: &MediaFile) -> &'static str {
    match f.container.to_lowercase().as_str() {
        "mkv" => "video/x-matroska",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "ts" | "m2ts" | "mts" => "video/mp2t",
        "webm" => "video/webm",
        "wmv" | "asf" => "video/x-ms-wmv",
        "mpg" | "mpeg" | "vob" => "video/mpeg",
        "flv" => "video/x-flv",
        "3gp" => "video/3gpp",
        _ => "video/mpeg",
    }
}

/// `H:MM:SS.mmm`.
fn duration(ms: i64) -> String {
    let ms = ms.max(0);
    format!("{}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Whether two language codes name the same language: ISO 639-1 ("de") and both forms of
/// ISO 639-2 ("ger", "deu") for the common languages.
fn same_language(a: &str, b: &str) -> bool {
    const CODES: &[&[&str]] = &[
        &["de", "ger", "deu"],
        &["en", "eng"],
        &["fr", "fre", "fra"],
        &["es", "spa"],
        &["it", "ita"],
        &["nl", "dut", "nld"],
        &["pl", "pol"],
        &["ru", "rus"],
        &["ja", "jpn"],
        &["tr", "tur"],
        &["pt", "por"],
        &["sv", "swe"],
        &["da", "dan"],
        &["no", "nor", "nob"],
        &["fi", "fin"],
        &["cs", "cze", "ces"],
        &["hu", "hun"],
        &["el", "gre", "ell"],
        &["zh", "chi", "zho"],
        &["ko", "kor"],
        &["ar", "ara"],
    ];
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    a == b || CODES.iter().any(|g| g.contains(&a.as_str()) && g.contains(&b.as_str()))
}

/// The text subtitles of a file: (FreeX id, language).
pub fn text_subtitles(f: &MediaFile) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> =
        f.subtitle_streams.iter().filter(|s| s.text_based()).map(|s| (format!("e{}", s.stream_index), s.language.clone())).collect();
    out.extend(f.subtitles.iter().enumerate().map(|(i, s)| (format!("x{i}"), s.language.clone())));
    out
}

/// What an object id names.
enum Object {
    Root,
    Resume,
    Recent,
    Library(Library),
    Item(Arc<Item>),
}

/// One entry of a folder.
enum Entry {
    Folder { id: String, title: String, count: usize, art: Option<String>, class: &'static str },
    Video { it: Arc<Item>, title: String },
}

pub struct Tree<'a> {
    app: &'a AppState,
    idx: &'a Index,
    view: View<'a>,
    user: &'a User,
    /// `http://host:port` as the player reached the server.
    base: String,
    german: bool,
}

impl<'a> Tree<'a> {
    pub fn new(app: &'a AppState, idx: &'a Index, user: &'a User, base: String) -> Tree<'a> {
        let german = match user.ui_language.as_str() {
            "de" => true,
            "" => app.config.tmdb.language.starts_with("de"),
            _ => false,
        };
        Tree { app, idx, view: View::new(app, idx, user), user, base, german }
    }

    fn label(&self, en: &str, de: &str) -> String {
        if self.german { de.into() } else { en.into() }
    }

    fn libraries(&self) -> Vec<Library> {
        self.app.db.libraries().into_iter().filter(|l| self.user.may_access(&l.id)).collect()
    }

    fn resolve(&self, id: &str) -> Option<Object> {
        match id {
            "0" => Some(Object::Root),
            "resume" => Some(Object::Resume),
            "recent" => Some(Object::Recent),
            _ => {
                if let Some(l) = id.strip_prefix("lib:") {
                    return self.libraries().into_iter().find(|x| x.id == l).map(Object::Library);
                }
                let it = self.idx.item(id.strip_prefix("item:")?)?;
                (self.view.sees(it) && it.kind != Kind::Extra).then(|| Object::Item(it.clone()))
            }
        }
    }

    /// The parent folder of an object, for BrowseMetadata.
    fn parent_of(&self, id: &str) -> String {
        match self.resolve(id) {
            Some(Object::Root) | None => "-1".into(),
            Some(Object::Resume | Object::Recent | Object::Library(_)) => "0".into(),
            Some(Object::Item(it)) => match it.kind {
                Kind::Movie | Kind::Show => format!("lib:{}", it.library_id),
                _ => self.idx.parent(&it).map_or_else(|| format!("lib:{}", it.library_id), |p| format!("item:{}", p.id)),
            },
        }
    }

    fn started(&self) -> Vec<Arc<Item>> {
        self.view.started(None).into_iter().take(ROW).collect()
    }

    fn recent(&self) -> Vec<Arc<Item>> {
        let mut list: Vec<&Arc<Item>> =
            self.idx.all().filter(|it| matches!(it.kind, Kind::Movie | Kind::Episode) && it.file.is_some() && self.view.sees(it)).collect();
        list.sort_by_key(|it| std::cmp::Reverse(it.added_at));
        list.into_iter().take(ROW).cloned().collect()
    }

    /// "Show – S1 E2 – Title" outside the show's own folders.
    fn long_title(&self, it: &Item) -> String {
        match (it.kind, self.idx.show_of(it), self.idx.parent(it)) {
            (Kind::Episode, Some(show), Some(season)) => format!("{} – S{} E{} – {}", show.title, season.index, it.index, it.title),
            _ => it.title.clone(),
        }
    }

    fn folder(&self, id: String, title: String, count: usize, art: Option<&Item>, class: &'static str) -> Entry {
        Entry::Folder { id, title, count, art: art.map(|a| format!("{}/dlna/image/{}", self.base, a.id)), class }
    }

    fn children(&self, obj: &Object) -> Vec<Entry> {
        match obj {
            Object::Root => {
                let mut out = Vec::new();
                let started = self.started().len();
                if started > 0 {
                    out.push(self.folder(
                        "resume".into(),
                        self.label("Continue watching", "Weiterschauen"),
                        started,
                        None,
                        "object.container.storageFolder",
                    ));
                }
                out.push(self.folder(
                    "recent".into(),
                    self.label("Recently added", "Neu hinzugefügt"),
                    self.recent().len(),
                    None,
                    "object.container.storageFolder",
                ));
                for l in self.libraries() {
                    let n = self.idx.top_level(&l.id).iter().filter(|it| self.view.sees(it)).count();
                    out.push(self.folder(format!("lib:{}", l.id), l.name.clone(), n, None, "object.container.storageFolder"));
                }
                out
            }
            Object::Resume => self.started().into_iter().map(|it| Entry::Video { title: self.long_title(&it), it }).collect(),
            Object::Recent => self.recent().into_iter().map(|it| Entry::Video { title: self.long_title(&it), it }).collect(),
            Object::Library(l) => {
                self.idx.top_level(&l.id).iter().filter(|it| self.view.sees(it)).filter_map(|it| self.entry(it)).collect()
            }
            Object::Item(it) => self.idx.children(&it.id).iter().filter(|c| self.view.sees(c)).filter_map(|c| self.entry(c)).collect(),
        }
    }

    /// A movie or episode as video, a show or season as folder.
    fn entry(&self, it: &Arc<Item>) -> Option<Entry> {
        match it.kind {
            Kind::Movie => it.file.as_ref().map(|_| Entry::Video { it: it.clone(), title: it.title.clone() }),
            Kind::Episode => it.file.as_ref().map(|_| Entry::Video { it: it.clone(), title: format!("{}. {}", it.index, it.title) }),
            Kind::Show | Kind::Season => {
                let n = self.idx.children(&it.id).iter().filter(|c| self.view.sees(c)).count();
                let title = if it.kind == Kind::Season && it.title.is_empty() {
                    self.label(&format!("Season {}", it.index), &format!("Staffel {}", it.index))
                } else {
                    it.title.clone()
                };
                Some(self.folder(format!("item:{}", it.id), title, n, Some(it), "object.container.storageFolder"))
            }
            Kind::Extra => None,
        }
    }

    fn entry_id(e: &Entry) -> String {
        match e {
            Entry::Folder { id, .. } => id.clone(),
            Entry::Video { it, .. } => format!("item:{}", it.id),
        }
    }

    fn didl(&self, entries: &[(Entry, String)]) -> String {
        let mut s = String::from(
            r#"<DIDL-Lite xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/" xmlns:dlna="urn:schemas-dlna-org:metadata-1-0/" xmlns:sec="http://www.sec.co.kr/">"#,
        );
        for (e, parent) in entries {
            match e {
                Entry::Folder { id, title, count, art, class } => {
                    s.push_str(&format!(
                        r#"<container id="{}" parentID="{}" restricted="1" searchable="0" childCount="{count}"><dc:title>{}</dc:title><upnp:class>{class}</upnp:class>"#,
                        esc(id),
                        esc(parent),
                        esc(title)
                    ));
                    if let Some(a) = art {
                        s.push_str(&format!(r#"<upnp:albumArtURI dlna:profileID="JPEG_TN">{}</upnp:albumArtURI>"#, esc(a)));
                    }
                    s.push_str("</container>");
                }
                Entry::Video { it, title } => s.push_str(&self.video(it, title, parent)),
            }
        }
        s.push_str("</DIDL-Lite>");
        s
    }

    fn video(&self, it: &Item, title: &str, parent: &str) -> String {
        let Some(f) = &it.file else { return String::new() };
        let base = &self.base;
        let mut s = format!(
            r#"<item id="item:{}" parentID="{}" restricted="1"><dc:title>{}</dc:title><upnp:class>{}</upnp:class>"#,
            esc(&it.id),
            esc(parent),
            esc(title),
            if it.kind == Kind::Movie { "object.item.videoItem.movie" } else { "object.item.videoItem" }
        );
        if it.year > 0 {
            s.push_str(&format!("<dc:date>{:04}-01-01</dc:date>", it.year));
        }
        for g in it.genre_list() {
            s.push_str(&format!("<upnp:genre>{}</upnp:genre>", esc(g)));
        }
        if !it.summary.is_empty() {
            s.push_str(&format!("<dc:description>{0}</dc:description><upnp:longDescription>{0}</upnp:longDescription>", esc(&it.summary)));
        }
        if let Some(show) = self.idx.show_of(it) {
            s.push_str(&format!(
                "<upnp:seriesTitle>{}</upnp:seriesTitle><upnp:episodeNumber>{}</upnp:episodeNumber>",
                esc(&show.title),
                it.index
            ));
        }
        s.push_str(&format!(r#"<upnp:albumArtURI dlna:profileID="JPEG_TN">{base}/dlna/image/{}</upnp:albumArtURI>"#, esc(&it.id)));
        // Samsung: resume position in seconds.
        let p = self.view.progress(it.key());
        if p.offset_ms > 0 && !p.watched {
            s.push_str(&format!("<sec:dcmInfo>CREATIONDATE=0,BM={}</sec:dcmInfo>", p.offset_ms / 1000));
        }
        let subs = text_subtitles(f);
        if let Some((sid, _)) = self.preferred_subtitle(&subs) {
            s.push_str(&format!(r#"<sec:CaptionInfoEx sec:type="srt">{base}/dlna/subtitle/{}/{sid}.srt</sec:CaptionInfoEx>"#, esc(&it.id)));
        }
        let mut attrs =
            format!(r#" protocolInfo="http-get:*:{}:{FLAGS}" size="{}" duration="{}""#, mime(f), f.size, duration(f.duration_ms));
        if f.width > 0 {
            attrs.push_str(&format!(r#" resolution="{}x{}""#, f.width, f.height));
        }
        if f.bitrate > 0 {
            // UPnP counts bytes per second.
            attrs.push_str(&format!(r#" bitrate="{}""#, f.bitrate / 8));
        }
        if let Some(a) = f.audio_streams.first() {
            attrs.push_str(&format!(r#" nrAudioChannels="{}""#, a.channels));
        }
        s.push_str(&format!("<res{attrs}>{base}/dlna/media/{}</res>", esc(&it.id)));
        for (sid, _) in &subs {
            s.push_str(&format!(r#"<res protocolInfo="http-get:*:text/srt:*">{base}/dlna/subtitle/{}/{sid}.srt</res>"#, esc(&it.id)));
        }
        s.push_str("</item>");
        s
    }

    /// The subtitle in the user's subtitle language, else the first; none if subtitles are off.
    pub fn preferred_subtitle<'s>(&self, subs: &'s [(String, String)]) -> Option<&'s (String, String)> {
        if self.user.subtitle_mode == "off" {
            return None;
        }
        let lang = &self.user.subtitle_language;
        subs.iter().find(|(_, l)| !lang.is_empty() && same_language(l, lang)).or(subs.first())
    }

    /// `BrowseDirectChildren`: DIDL, entries returned, entries in total. None for unknown ids.
    pub fn browse_children(&self, id: &str, start: usize, count: usize) -> Option<(String, usize, usize)> {
        let obj = self.resolve(id)?;
        let all = self.children(&obj);
        let total = all.len();
        let page: Vec<(Entry, String)> =
            all.into_iter().skip(start).take(if count == 0 { usize::MAX } else { count }).map(|e| (e, id.to_string())).collect();
        let n = page.len();
        Some((self.didl(&page), n, total))
    }

    /// `BrowseMetadata`: the object itself.
    pub fn browse_metadata(&self, id: &str) -> Option<String> {
        let parent = self.parent_of(id);
        let entry = match self.resolve(id)? {
            Object::Root => Entry::Folder {
                id: "0".into(),
                title: self.app.config.server.name.clone(),
                count: self.children(&Object::Root).len(),
                art: None,
                class: "object.container.storageFolder",
            },
            obj @ (Object::Resume | Object::Recent | Object::Library(_)) => {
                let count = self.children(&obj).len();
                let title = match &obj {
                    Object::Resume => self.label("Continue watching", "Weiterschauen"),
                    Object::Recent => self.label("Recently added", "Neu hinzugefügt"),
                    Object::Library(l) => l.name.clone(),
                    _ => unreachable!(),
                };
                Entry::Folder { id: id.to_string(), title, count, art: None, class: "object.container.storageFolder" }
            }
            Object::Item(it) => self.entry(&it)?,
        };
        debug_assert_eq!(Self::entry_id(&entry), id);
        Some(self.didl(&[(entry, parent)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration(5_025_123), "1:23:45.123");
        assert_eq!(duration(59_000), "0:00:59.000");
    }

    #[test]
    fn languages() {
        assert!(same_language("ger", "de") && same_language("DEU", "ger") && same_language("eng", "eng"));
        assert!(!same_language("eng", "de"));
    }

    #[test]
    fn types() {
        let f = |c: &str| MediaFile { container: c.into(), ..Default::default() };
        assert_eq!(mime(&f("mkv")), "video/x-matroska");
        assert_eq!(mime(&f("MP4")), "video/mp4");
        assert_eq!(mime(&f("ts")), "video/mp2t");
    }
}
