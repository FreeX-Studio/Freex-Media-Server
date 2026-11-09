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

//! Deleting titles from disk (admins, `[library] allow_delete`). What goes: the video files,
//! their subtitles and the artwork and `.nfo` named after them, and the extras of a movie. A
//! folder is removed only when nothing else is left in it, so other files are never touched.

use super::catalog::Index;
use super::{Item, Kind};
use std::path::{Path, PathBuf};

/// Files next to a video that belong to it: `<base>.*` subtitles, artwork and `.nfo`.
const SIDECAR_SUFFIXES: [&str; 11] =
    [".nfo", ".srt", ".vtt", ".jpg", "-poster.jpg", "-fanart.jpg", "-thumb.jpg", ".png", "-poster.png", "-fanart.png", "-thumb.png"];

/// The files that deleting `it` removes, in a fixed order. `version` limits a movie with several
/// versions to that one file.
pub fn files_of(idx: &Index, it: &Item, version: Option<&str>) -> Vec<PathBuf> {
    let mut videos: Vec<PathBuf> = Vec::new();
    let mut extras = true;
    match it.kind {
        Kind::Movie | Kind::Episode | Kind::Extra => {
            if let Some(v) = version.and_then(|v| it.versions.iter().find(|x| x.id == v)) {
                videos.push(PathBuf::from(&v.file.path));
                // Other versions stay, and with them the extras.
                extras = it.versions.len() <= 1;
            } else if it.versions.is_empty() {
                videos.extend(it.file.iter().map(|f| PathBuf::from(&f.path)));
            } else {
                videos.extend(it.versions.iter().map(|v| PathBuf::from(&v.file.path)));
            }
        }
        Kind::Season => videos.extend(idx.children(&it.id).iter().filter_map(|e| e.file.as_ref()).map(|f| PathBuf::from(&f.path))),
        Kind::Show => videos.extend(idx.episodes_of(it).iter().filter_map(|e| e.file.as_ref()).map(|f| PathBuf::from(&f.path))),
    }
    let mut out = Vec::new();
    for v in &videos {
        out.push(v.clone());
        out.extend(sidecars(v));
    }
    if extras && matches!(it.kind, Kind::Movie | Kind::Show) {
        for e in idx.extras(&it.id) {
            if let Some(f) = &e.file {
                out.push(PathBuf::from(&f.path));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Subtitles, artwork and `.nfo` named like the video (`Movie.de.srt` included).
fn sidecars(video: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().map(|s| s.to_string_lossy().to_lowercase())) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p != video && p.is_file())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let Some(rest) = name.strip_prefix(&stem) else { return false };
            // "Movie.de.forced.srt": a language part before the subtitle extension.
            SIDECAR_SUFFIXES.contains(&rest)
                || (rest.starts_with('.') && (rest.ends_with(".srt") || rest.ends_with(".vtt")) && rest.len() <= 20)
        })
        .collect()
}

/// Deletes the files; then every folder they were in that is empty now, up to (not including)
/// the library folders. Returns the number of files deleted, or the first error.
pub fn delete(files: &[PathBuf], library_roots: &[String]) -> Result<usize, String> {
    let mut n = 0;
    for f in files {
        match std::fs::remove_file(f) {
            Ok(()) => n += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", f.display())),
        }
    }
    let roots: Vec<PathBuf> = library_roots.iter().map(PathBuf::from).collect();
    let mut dirs: Vec<PathBuf> = files.iter().filter_map(|f| f.parent().map(Path::to_path_buf)).collect();
    // Deepest first, so that "Show/Season 1" goes before "Show".
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    dirs.dedup();
    for mut d in dirs {
        while !roots.contains(&d) && roots.iter().any(|r| d.starts_with(r)) {
            // remove_dir only removes empty folders.
            if std::fs::remove_dir(&d).is_err() {
                break;
            }
            match d.parent() {
                Some(p) => d = p.to_path_buf(),
                None => break,
            }
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletes_a_movie_and_what_belongs_to_it() {
        let root = std::env::temp_dir().join(format!("freex-delete-{}", std::process::id()));
        let movie = root.join("Inception (2010)");
        std::fs::create_dir_all(movie.join("Trailers")).unwrap();
        let video = movie.join("Inception (2010).mkv");
        for f in ["Inception (2010).mkv", "Inception (2010).de.forced.srt", "Inception (2010).nfo", "Inception (2010)-poster.jpg"] {
            std::fs::write(movie.join(f), b"x").unwrap();
        }
        std::fs::write(movie.join("Trailers/Trailer.mkv"), b"x").unwrap();
        std::fs::write(root.join("Other.mkv"), b"x").unwrap();
        let mut files = vec![video.clone()];
        files.extend(sidecars(&video));
        files.push(movie.join("Trailers/Trailer.mkv"));
        assert_eq!(files.len(), 5);
        assert_eq!(delete(&files, &[root.to_string_lossy().into_owned()]).unwrap(), 5);
        // The emptied folders are gone, the library folder and other files stay.
        assert!(!movie.exists());
        assert!(root.join("Other.mkv").exists());
        // A folder with something else in it stays.
        let keep = root.join("Keep (2000)");
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::write(keep.join("Keep (2000).mkv"), b"x").unwrap();
        std::fs::write(keep.join("notes.txt"), b"x").unwrap();
        delete(&[keep.join("Keep (2000).mkv")], &[root.to_string_lossy().into_owned()]).unwrap();
        assert!(keep.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
