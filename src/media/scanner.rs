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

//! Library scans: walks the folders, builds movies or shows/seasons/episodes from the names,
//! `.nfo` files and artwork next to the videos, and probes new or changed files with ffprobe.
//!
//! Scans run one after another in the background. Ids are derived from the file paths, so watch
//! state survives rescans; unchanged files keep their probe data. A folder that is missing (an
//! unmounted disk) keeps its old entries instead of emptying the library.

use super::names::{self, sort_title, title_year};
use super::{Item, Kind, MediaFile, Subtitle, nfo, probe};
use crate::AppState;
use crate::auth::{now_ms, sha256};
use crate::db::Library;
use crate::metadata::matching::{imdb_tag, tvdb_tag};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tokio::task::JoinSet;

pub const VIDEO_EXTENSIONS: [&str; 15] =
    ["mkv", "mp4", "m4v", "webm", "avi", "mov", "ts", "m2ts", "mts", "wmv", "mpg", "mpeg", "flv", "ogv", "3gp"];
const IMAGE_EXTENSIONS: [&str; 4] = ["jpg", "jpeg", "png", "webp"];
const SUBTITLE_EXTENSIONS: [&str; 2] = ["srt", "vtt"];
/// Folders that hold bonus material or system files, not main content.
const SKIP_FOLDERS: [&str; 18] = [
    "extras",
    "featurettes",
    "trailers",
    "behind the scenes",
    "deleted scenes",
    "interviews",
    "scenes",
    "shorts",
    "samples",
    "sample",
    "other",
    "@eadir",
    "#recycle",
    "$recycle.bin",
    "lost+found",
    ".actors",
    "metadata",
    "system volume information",
];
/// Smaller files named "sample" or "-trailer" are not main content.
const SAMPLE_MAX_BYTES: u64 = 300 * 1024 * 1024;

/// Stable 16-character id from the given parts.
pub fn stable_id(parts: &[&str]) -> String {
    sha256(&parts.join("\0"))[..16].to_string()
}

#[derive(Default)]
pub struct Scanner {
    /// Libraries queued or being scanned.
    scanning: Mutex<HashSet<String>>,
    /// One scan at a time: they compete for the same disks.
    serial: tokio::sync::Mutex<()>,
}

impl Scanner {
    pub fn is_scanning(&self, library_id: &str) -> bool {
        self.scanning.lock().unwrap().contains(library_id)
    }

    /// Queues a scan of a library; does nothing if one is already queued or running.
    pub fn scan(app: &Arc<AppState>, library_id: &str) {
        if !app.scanner.scanning.lock().unwrap().insert(library_id.to_string()) {
            return;
        }
        let app = app.clone();
        let id = library_id.to_string();
        tokio::spawn(async move {
            {
                let _turn = app.scanner.serial.lock().await;
                // Read now: the library may have been changed or deleted while the scan waited.
                if let Some(lib) = app.db.library(&id) {
                    scan_now(&app, lib).await;
                }
            }
            app.scanner.scanning.lock().unwrap().remove(&id);
        });
    }

    pub fn scan_all(app: &Arc<AppState>) {
        for l in app.db.libraries() {
            Scanner::scan(app, &l.id);
        }
    }

    /// Rescans all libraries every `minutes` (0: never).
    pub fn spawn_timer(app: Arc<AppState>, minutes: u32) {
        if minutes == 0 {
            return;
        }
        tokio::spawn(async move {
            let every = Duration::from_secs(u64::from(minutes) * 60);
            loop {
                tokio::time::sleep(every).await;
                Scanner::scan_all(&app);
            }
        });
    }
}

async fn scan_now(app: &Arc<AppState>, lib: Library) {
    let start = Instant::now();
    let previous: HashMap<String, Arc<Item>> = app.catalog.snapshot().of_library(&lib.id).map(|it| (it.id.clone(), it.clone())).collect();
    let probe_ok = app.tools.ffprobe_ok;
    let walk_lib = lib.clone();
    let built = tokio::task::spawn_blocking(move || Build::run(&walk_lib, previous, probe_ok)).await;
    let (mut items, probes) = match built {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Scan of library '{}' failed: {e}", lib.name);
            return;
        }
    };

    // Probe new and changed files, a few at a time.
    let probed = probes.len();
    if probe_ok && !probes.is_empty() {
        let limit = Arc::new(tokio::sync::Semaphore::new(app.config.scanner.threads as usize));
        let mut tasks = JoinSet::new();
        for (id, path) in probes {
            let (limit, ffprobe) = (limit.clone(), app.tools.ffprobe.clone());
            tasks.spawn(async move {
                let _slot = limit.acquire_owned().await.ok()?;
                probe::probe(&ffprobe, &path).await.map(|j| (id, j))
            });
        }
        let pos: HashMap<String, usize> = items.iter().enumerate().map(|(i, it)| (it.id.clone(), i)).collect();
        while let Some(r) = tasks.join_next().await {
            if let Ok(Some((id, json))) = r
                && let Some(f) = pos.get(&id).and_then(|&i| items[i].file.as_mut())
            {
                probe::apply(&json, f);
            }
        }
    }

    finish_shows(&lib, &mut items);
    app.metadata.apply_all(&mut items);
    let count = items.len();
    app.catalog.replace_library(&app.db, &lib.id, items);
    app.db.set_last_scan(&lib.id, now_ms());
    // New entries, renamed ones and old metadata are fetched in the background.
    crate::metadata::Service::refresh_library(app, &lib.id, false);
    crate::markers::Service::analyze_library(app, &lib.id, false);
    tracing::info!("Scanned library '{}': {count} entries, {probed} files probed, {} ms", lib.name, start.elapsed().as_millis());
}

/// Shows and seasons count as "added" when their newest episode was added.
fn finish_shows(lib: &Library, items: &mut [Item]) {
    if lib.kind != "shows" {
        return;
    }
    let mut newest: HashMap<String, i64> = HashMap::new();
    for it in items.iter().filter(|it| it.kind == Kind::Episode) {
        if let Some(p) = &it.parent_id {
            let e = newest.entry(p.clone()).or_insert(it.added_at);
            *e = (*e).max(it.added_at);
        }
    }
    for it in items.iter_mut().filter(|it| it.kind == Kind::Season) {
        it.added_at = newest.get(&it.id).copied().unwrap_or(it.added_at);
        if let Some(p) = &it.parent_id {
            let e = newest.entry(p.clone()).or_insert(it.added_at);
            *e = (*e).max(it.added_at);
        }
    }
    for it in items.iter_mut().filter(|it| it.kind == Kind::Show) {
        it.added_at = newest.get(&it.id).copied().unwrap_or(it.added_at);
    }
}

/// The file system part of a scan; runs on a blocking thread.
struct Build {
    library_id: String,
    previous: HashMap<String, Arc<Item>>,
    first_scan: bool,
    probe_ok: bool,
    items: Vec<Item>,
    pos: HashMap<String, usize>,
    /// Files to probe: item id and path.
    probes: Vec<(String, PathBuf)>,
    /// Listings of the folders seen in this scan; one per folder, not one per episode.
    sidecars: HashMap<PathBuf, Rc<Sidecars>>,
}

type BuildResult = (Vec<Item>, Vec<(String, PathBuf)>);

impl Build {
    fn run(lib: &Library, previous: HashMap<String, Arc<Item>>, probe_ok: bool) -> BuildResult {
        let mut b = Build {
            library_id: lib.id.clone(),
            previous,
            first_scan: lib.last_scan == 0,
            probe_ok,
            items: Vec::new(),
            pos: HashMap::new(),
            probes: Vec::new(),
            sidecars: HashMap::new(),
        };
        for p in &lib.paths {
            let root = PathBuf::from(p);
            if !root.is_dir() {
                tracing::warn!("Library '{}': folder not found: {}", lib.name, root.display());
                // A missing folder (unmounted disk) must not wipe the index: keep its old entries.
                let keep: Vec<Item> = b.previous.values().filter(|it| belongs_to(it, p)).map(|it| Item::clone(it)).collect();
                for it in keep {
                    b.put(it);
                }
                continue;
            }
            for video in find_videos(&root) {
                let r = if lib.kind == "movies" { b.add_movie(&root, &video) } else { b.add_episode(&root, &video) };
                if let Err(e) = r {
                    tracing::warn!("Skipping {}: {e}", video.display());
                }
            }
        }
        (b.items, b.probes)
    }

    fn put(&mut self, it: Item) {
        match self.pos.get(&it.id) {
            Some(&i) => self.items[i] = it,
            None => {
                self.pos.insert(it.id.clone(), self.items.len());
                self.items.push(it);
            }
        }
    }

    fn get(&self, id: &str) -> Option<&Item> {
        self.pos.get(id).map(|&i| &self.items[i])
    }

    fn sidecars(&mut self, dir: &Path) -> Rc<Sidecars> {
        self.sidecars.entry(dir.to_path_buf()).or_insert_with(|| Rc::new(Sidecars::read(dir))).clone()
    }

    fn add_movie(&mut self, root: &Path, video: &Path) -> std::io::Result<()> {
        let abs = text(video);
        let id = stable_id(&[&self.library_id, "movie", &abs]);
        let mut item = Item::new(id.clone(), &self.library_id, Kind::Movie);
        let dir = video.parent().unwrap_or(root);
        let file_name = file_name(video);
        let base = names::strip_extension(&file_name).to_string();
        let mut ty = title_year(&base);
        // Generic file names ("movie.mkv", "1080p.mkv") get their title from the folder.
        if dir != root && (ty.year == 0 || ty.title.chars().count() <= 2) {
            let folder_ty = title_year(&file_name_of(dir));
            if folder_ty.year != 0 || ty.title.chars().count() <= 2 {
                ty = folder_ty;
            }
        }
        item.sort_title = sort_title(&ty.title);
        item.title = ty.title;
        item.year = ty.year;
        item.folder = Some(text(dir));
        let sc = self.sidecars(dir);
        let own_folder = dir != root && sc.video_count <= 2;
        let nfo = sc.find(&format!("{base}.nfo")).or_else(|| if own_folder { sc.find("movie.nfo") } else { None });
        if let Some(n) = nfo {
            nfo::apply(n, &mut item);
        }
        if item.tmdb_hint == 0 {
            item.tmdb_hint = names::tmdb_tag(&file_name);
            if item.tmdb_hint == 0 && own_folder {
                item.tmdb_hint = names::tmdb_tag(&file_name_of(dir));
            }
        }
        if item.imdb_hint.is_empty() {
            let folder = if own_folder { file_name_of(dir) } else { String::new() };
            item.imdb_hint = imdb_tag(&file_name).or_else(|| imdb_tag(&folder)).unwrap_or_default();
        }
        item.poster_path = sc.image(&[&format!("{base}-poster"), &format!("{base}.poster"), &base]);
        if item.poster_path.is_none() && own_folder {
            item.poster_path = sc.image(&["poster", "folder", "cover", "movie"]);
        }
        item.backdrop_path = sc.image(&[&format!("{base}-fanart"), &format!("{base}.fanart"), &format!("{base}-backdrop")]);
        if item.backdrop_path.is_none() && own_folder {
            item.backdrop_path = sc.image(&["fanart", "backdrop", "background", "art"]);
        }
        item.file = Some(self.file_info(&id, video, &sc, &base)?);
        item.added_at = self.added_at(&id, video)?;
        self.put(item);
        Ok(())
    }

    fn add_episode(&mut self, root: &Path, video: &Path) -> std::io::Result<()> {
        let rel: Vec<String> = video.strip_prefix(root).unwrap_or(video).iter().map(|c| c.to_string_lossy().into_owned()).collect();
        let file_name = file_name(video);
        let base = names::strip_extension(&file_name).to_string();
        let dir = video.parent().unwrap_or(root);
        // The show is the first folder below the library root, the season optionally the next one.
        let show_dir = (rel.len() >= 2).then(|| root.join(&rel[0]));
        let folder_season = if rel.len() >= 3 { names::season_from_folder(&rel[1]) } else { -1 };
        let ep = names::episode(&file_name, folder_season);
        let (show_key, show_title_raw) = match &show_dir {
            Some(d) => (text(d), file_name_of(d)),
            None => {
                // Episode directly in the root: the show name is the part before SxxEyy.
                let raw = if ep.show_part.is_empty() { base.clone() } else { ep.show_part.clone() };
                (format!("{}/{}", text(root), sort_title(&raw)), raw)
            }
        };
        let season = if ep.season >= 0 { ep.season } else { folder_season.max(1) };
        let episode = ep.episode;

        let show_id = stable_id(&[&self.library_id, "show", &show_key]);
        if self.get(&show_id).is_none() {
            let mut show = Item::new(show_id.clone(), &self.library_id, Kind::Show);
            let ty = title_year(&show_title_raw);
            show.sort_title = sort_title(&ty.title);
            show.title = ty.title;
            show.year = ty.year;
            show.folder = show_dir.as_deref().map(text);
            if let Some(d) = &show_dir {
                let ssc = self.sidecars(d);
                if let Some(n) = ssc.find("tvshow.nfo") {
                    nfo::apply(n, &mut show);
                }
                if show.tmdb_hint == 0 {
                    show.tmdb_hint = names::tmdb_tag(&show_title_raw);
                }
                if show.imdb_hint.is_empty() {
                    show.imdb_hint = imdb_tag(&show_title_raw).unwrap_or_default();
                }
                if show.tvdb_hint == 0 {
                    show.tvdb_hint = tvdb_tag(&show_title_raw).unwrap_or(0);
                }
                show.poster_path = ssc.image(&["poster", "folder", "cover", "show"]);
                show.backdrop_path = ssc.image(&["fanart", "backdrop", "background", "art"]);
            }
            show.added_at = self.previous.get(&show_id).map_or(0, |o| o.added_at);
            self.put(show);
        }
        let (show_title, show_year) = {
            let s = self.get(&show_id).expect("just added");
            (s.title.clone(), s.year)
        };

        let season_id = stable_id(&[&self.library_id, "season", &show_key, &season.to_string()]);
        if self.get(&season_id).is_none() {
            let mut s = Item::new(season_id.clone(), &self.library_id, Kind::Season);
            s.parent_id = Some(show_id.clone());
            s.index = season;
            s.title = if season == 0 { "Specials".into() } else { format!("Staffel {season}") };
            s.sort_title = format!("{season:04}");
            s.year = show_year;
            if folder_season >= 0 && show_dir.as_deref() != Some(dir) {
                s.folder = Some(text(dir));
                s.poster_path = self.sidecars(dir).image(&["poster", "folder", "cover"]);
            }
            if s.poster_path.is_none()
                && let Some(d) = &show_dir
            {
                let key = if season == 0 { "season-specials".to_string() } else { format!("season{season:02}") };
                s.poster_path = self.sidecars(d).image(&[&format!("{key}-poster"), &key]);
            }
            s.added_at = self.previous.get(&season_id).map_or(0, |o| o.added_at);
            self.put(s);
        }

        let abs = text(video);
        let id = stable_id(&[&self.library_id, "episode", &abs]);
        let mut item = Item::new(id.clone(), &self.library_id, Kind::Episode);
        item.parent_id = Some(season_id);
        item.index = episode.max(0);
        let label = if episode >= 0 {
            if ep.episode_end > 0 { format!("Episode {episode}–{}", ep.episode_end) } else { format!("Episode {episode}") }
        } else {
            base.clone()
        };
        item.title = if ep.title.is_empty() || ep.title.eq_ignore_ascii_case(&show_title) { label } else { ep.title.clone() };
        item.sort_title = format!("{:04} {}", item.index, sort_title(&item.title));
        item.year = show_year;
        let sc = self.sidecars(dir);
        if let Some(n) = sc.find(&format!("{base}.nfo")) {
            nfo::apply(n, &mut item);
            item.sort_title = format!("{:04} {}", item.index, sort_title(&item.title));
        }
        item.poster_path = sc.image(&[&format!("{base}-thumb"), &base]);
        item.file = Some(self.file_info(&id, video, &sc, &base)?);
        item.added_at = self.added_at(&id, video)?;
        self.put(item);
        Ok(())
    }

    /// The file's data: kept from the last scan if the file is unchanged, otherwise probed again.
    fn file_info(&mut self, id: &str, video: &Path, sc: &Sidecars, base: &str) -> std::io::Result<MediaFile> {
        let meta = std::fs::metadata(video)?;
        let size = meta.len() as i64;
        let modified = mtime_ms(&meta);
        let old = self.previous.get(id).and_then(|o| o.file.as_ref());
        let mut f = match old {
            Some(o) if o.size == size && o.modified == modified && (o.probe_current() || !self.probe_ok) => o.clone(),
            _ => {
                self.probes.push((id.to_string(), video.to_path_buf()));
                MediaFile { path: text(video), size, modified, container: names::extension(&file_name(video)), ..MediaFile::default() }
            }
        };
        f.subtitles = sc.subtitles(base);
        Ok(f)
    }

    /// Keeps the original "added" time; on the first scan the file date is used, so the order makes sense.
    fn added_at(&self, id: &str, video: &Path) -> std::io::Result<i64> {
        if let Some(t) = self.previous.get(id).map(|o| o.added_at).filter(|t| *t > 0) {
            return Ok(t);
        }
        let now = now_ms();
        if self.first_scan { Ok(now.min(mtime_ms(&std::fs::metadata(video)?))) } else { Ok(now) }
    }
}

fn belongs_to(it: &Item, root: &str) -> bool {
    match (&it.file, &it.folder) {
        (Some(f), _) => f.path.starts_with(root),
        (None, Some(folder)) => folder.starts_with(root),
        _ => false,
    }
}

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn file_name_of(dir: &Path) -> String {
    file_name(dir)
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64)
}

/// Video files below a folder, sorted, without hidden files, bonus folders and samples.
/// Symbolic links to files are followed, links to folders are not (they could form loops).
fn find_videos(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!("Cannot read {}: {e}", dir.display());
                continue;
            }
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                if !SKIP_FOLDERS.contains(&name.to_lowercase().as_str()) && !path.join(".ignore").exists() {
                    stack.push(path);
                }
                continue;
            }
            let ext = names::extension(&name);
            if !VIDEO_EXTENSIONS.contains(&ext.as_str()) {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            if !meta.is_file() {
                continue;
            }
            let lower = name.to_lowercase();
            if meta.len() < SAMPLE_MAX_BYTES && (lower.contains("sample") || lower.ends_with(&format!("-trailer.{ext}"))) {
                continue;
            }
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Listing of one folder, to find artwork, `.nfo` files and subtitles next to a video.
#[derive(Default)]
struct Sidecars {
    by_lower_name: HashMap<String, PathBuf>,
    video_count: usize,
}

impl Sidecars {
    fn read(dir: &Path) -> Sidecars {
        let mut s = Sidecars::default();
        match std::fs::read_dir(dir) {
            Ok(entries) => {
                for e in entries.flatten() {
                    let n = e.file_name().to_string_lossy().into_owned();
                    if VIDEO_EXTENSIONS.contains(&names::extension(&n).as_str()) {
                        s.video_count += 1;
                    }
                    s.by_lower_name.insert(n.to_lowercase(), e.path());
                }
            }
            Err(e) => tracing::debug!("Cannot list {}: {e}", dir.display()),
        }
        s
    }

    fn find(&self, name: &str) -> Option<&Path> {
        self.by_lower_name.get(&name.to_lowercase()).map(PathBuf::as_path)
    }

    /// First existing image among the given base names (any supported image extension).
    fn image(&self, bases: &[&str]) -> Option<String> {
        bases.iter().flat_map(|b| IMAGE_EXTENSIONS.iter().map(move |e| format!("{b}.{e}"))).find_map(|n| self.find(&n).map(text))
    }

    /// Subtitles named `<base>.srt`, `<base>.<lang>.srt`, `<base>.<lang>.forced.srt`.
    fn subtitles(&self, base: &str) -> Vec<Subtitle> {
        let prefix = base.to_lowercase();
        let mut out: Vec<Subtitle> = Vec::new();
        for (n, path) in &self.by_lower_name {
            let ext = names::extension(n);
            if !SUBTITLE_EXTENSIONS.contains(&ext.as_str()) || !n.starts_with(&prefix) {
                continue;
            }
            let middle = &n[prefix.len()..n.len() - ext.len() - 1];
            if !middle.is_empty() && !middle.starts_with('.') {
                continue; // belongs to another video
            }
            let mut forced = false;
            let mut lang = String::new();
            for part in middle.split('.').filter(|p| !p.is_empty()) {
                match part {
                    "forced" => forced = true,
                    "sdh" | "cc" | "default" => {}
                    _ => lang = part.to_string(),
                }
            }
            out.push(Subtitle { path: text(path), language: lang, format: ext, forced });
        }
        out.sort_by(|a, b| a.language.cmp(&b.language).then_with(|| a.path.cmp(&b.path)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path, bytes: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![0u8; bytes]).unwrap();
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("freex-scan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn lib(kind: &str, root: &Path) -> Library {
        Library { id: "lib".into(), name: "Test".into(), kind: kind.into(), paths: vec![text(root)], created_at: 0, last_scan: 0 }
    }

    #[test]
    fn builds_movies() {
        let root = temp("movies");
        touch(&root.join("Inception (2010)/Inception (2010).mkv"), 10);
        touch(&root.join("Inception (2010)/poster.jpg"), 1);
        touch(&root.join("Inception (2010)/Inception (2010).de.forced.srt"), 1);
        touch(&root.join("Inception (2010)/Inception (2010)-sample.mkv"), 1);
        touch(&root.join("The.Matrix.1999.1080p.BluRay.mkv"), 10);
        touch(&root.join("Extras/Bonus.mkv"), 10);
        let (items, probes) = Build::run(&lib("movies", &root), HashMap::new(), true);
        let mut titles: Vec<_> = items.iter().map(|i| (i.title.as_str(), i.year)).collect();
        titles.sort();
        assert_eq!(titles, vec![("Inception", 2010), ("The Matrix", 1999)]);
        let inception = items.iter().find(|i| i.title == "Inception").unwrap();
        assert!(inception.poster_path.as_deref().unwrap().ends_with("poster.jpg"));
        let subs = &inception.file.as_ref().unwrap().subtitles;
        assert_eq!((subs.len(), subs[0].language.as_str(), subs[0].forced), (1, "de", true));
        assert_eq!(probes.len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn builds_shows_and_keeps_ids() {
        let root = temp("shows");
        touch(&root.join("Dark/Staffel 1/Dark.S01E01.Geheimnisse.mkv"), 10);
        touch(&root.join("Dark/Staffel 1/Dark.S01E02.Lügen.mkv"), 10);
        touch(&root.join("Dark/Specials/Dark.S00E01.Making.Of.mkv"), 10);
        touch(&root.join("Dark/poster.png"), 1);
        let l = lib("shows", &root);
        let (mut items, _) = Build::run(&l, HashMap::new(), true);
        finish_shows(&l, &mut items);
        let show = items.iter().find(|i| i.kind == Kind::Show).unwrap();
        assert_eq!(show.title, "Dark");
        assert!(show.poster_path.is_some());
        let seasons: Vec<_> = items.iter().filter(|i| i.kind == Kind::Season).map(|s| s.index).collect();
        assert_eq!(seasons.len(), 2);
        let e2 = items.iter().find(|i| i.kind == Kind::Episode && i.index == 2).unwrap();
        assert_eq!(e2.title, "Lügen");
        let newest = items.iter().filter(|i| i.kind == Kind::Episode).map(|i| i.added_at).max().unwrap();
        assert_eq!(show.added_at, newest);

        // A second scan keeps ids, added times and probe data.
        let mut previous = HashMap::new();
        for mut it in items.clone() {
            if let Some(f) = it.file.as_mut() {
                f.probed = true;
                f.probe_version = super::super::PROBE_VERSION;
            }
            previous.insert(it.id.clone(), Arc::new(it));
        }
        let mut l2 = l.clone();
        l2.last_scan = 1;
        let (again, probes) = Build::run(&l2, previous, true);
        assert!(probes.is_empty());
        assert_eq!(again.len(), items.len());
        assert!(again.iter().any(|i| i.id == e2.id && i.added_at == e2.added_at));
        let _ = std::fs::remove_dir_all(&root);
    }
}
