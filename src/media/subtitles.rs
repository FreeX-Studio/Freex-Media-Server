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

//! Subtitles as WebVTT for the player: files next to the video are converted on the fly, text
//! streams inside the video are extracted once with ffmpeg and cached in `<cache>/subtitles`.

use super::Item;
use super::probe::command;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_FILE: u64 = 10 * 1024 * 1024;
/// Reading a stream means reading the whole video; large files take a while.
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(180);

pub struct Subtitles {
    dir: PathBuf,
    ffmpeg: Option<String>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Subtitles {
    pub fn new(cache_dir: &Path, ffmpeg: Option<String>) -> Subtitles {
        let dir = cache_dir.join("subtitles");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("Cannot create {}: {e}", dir.display());
        }
        Subtitles { dir, ffmpeg, locks: Mutex::default() }
    }

    /// WebVTT for a subtitle id (`x<n>` file next to the video, `e<stream>` stream inside it).
    pub async fn vtt(&self, item: &Item, id: &str) -> Option<String> {
        let f = item.file.as_ref()?;
        let (kind, n) = (id.chars().next()?, id.get(1..)?.parse::<i32>().ok()?);
        match kind {
            'x' => {
                let s = f.subtitles.get(usize::try_from(n).ok()?)?;
                let path = PathBuf::from(&s.path);
                let meta = tokio::fs::metadata(&path).await.ok()?;
                if !meta.is_file() || meta.len() > MAX_FILE {
                    return None;
                }
                let text = read_text(&tokio::fs::read(&path).await.ok()?);
                Some(if s.format == "vtt" { text } else { srt_to_vtt(&text) })
            }
            'e' => {
                let s = f.subtitle_streams.iter().find(|s| s.stream_index == n).filter(|s| s.text_based())?;
                self.extract(item, s.stream_index).await
            }
            _ => None,
        }
    }

    async fn extract(&self, item: &Item, stream: i32) -> Option<String> {
        let f = item.file.as_ref()?;
        let key = format!("{}-{stream}-{:x}", item.id, f.modified);
        let out = self.dir.join(format!("{key}.vtt"));
        if let Ok(t) = tokio::fs::read_to_string(&out).await {
            return Some(t);
        }
        let ffmpeg = self.ffmpeg.as_deref()?;
        let lock = self.locks.lock().unwrap().entry(key.clone()).or_default().clone();
        let result = async {
            let _one = lock.lock().await;
            if let Ok(t) = tokio::fs::read_to_string(&out).await {
                return Some(t);
            }
            let tmp = self.dir.join(format!("{key}.tmp.vtt"));
            let mut c = command(ffmpeg);
            c.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
                .arg(&f.path)
                .args(["-map", &format!("0:{stream}"), "-c:s", "webvtt", "-f", "webvtt"])
                .arg(&tmp)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let ok = matches!(tokio::time::timeout(EXTRACT_TIMEOUT, c.status()).await, Ok(Ok(s)) if s.success());
            if !ok || tokio::fs::rename(&tmp, &out).await.is_err() {
                let _ = tokio::fs::remove_file(&tmp).await;
                return None;
            }
            tokio::fs::read_to_string(&out).await.ok()
        }
        .await;
        self.locks.lock().unwrap().remove(&key);
        result
    }
}

/// UTF-8, or Windows-1252 for the many older (German) subtitles in that encoding; without BOM.
pub fn read_text(data: &[u8]) -> String {
    let s = match std::str::from_utf8(data) {
        Ok(s) => s.to_string(),
        Err(_) => data.iter().map(|&b| windows_1252(b)).collect(),
    };
    s.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(s)
}

fn windows_1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•',
        '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    if (0x80..0xa0).contains(&b) { HIGH[(b - 0x80) as usize] } else { b as char }
}

pub fn srt_to_vtt(srt: &str) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for line in srt.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if line.contains("-->") {
            out.push_str(&line.replace(',', "."));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts() {
        let vtt = srt_to_vtt("1\r\n00:00:01,000 --> 00:00:02,500\r\nHallo, Welt\r\n");
        assert!(vtt.starts_with("WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.500\nHallo, Welt\n"));
        assert_eq!(read_text(b"\xef\xbb\xbfabc"), "abc");
        assert_eq!(read_text(b"Gr\xfc\xdfe \x80"), "Grüße €");
    }
}
