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

//! HTTP access to TMDB (API v3, with a v3 key or a v4 read access token) and to TVmaze, which
//! needs no key. Requests honour `HTTPS_PROXY`; at most four run at the same time, and rate limits
//! (429) and server errors are retried with a pause.

use crate::i18n::Text;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use tokio::sync::Semaphore;

const MAX_PARALLEL: usize = 4;
const USER_AGENT: &str = concat!("FreeX-Media-Server/", env!("CARGO_PKG_VERSION"));

/// A failure with a text for the user.
#[derive(Debug, Clone)]
pub struct MetaError(pub Text);

impl MetaError {
    pub fn new(key: &str) -> MetaError {
        MetaError(Text::new(key))
    }

    pub fn key_rejected(&self) -> bool {
        self.0.key == "error.tmdbKeyRejected"
    }
}

pub type MetaResult<T> = Result<T, MetaError>;

pub struct Tmdb {
    http: reqwest::Client,
    api: String,
    images: String,
    key: String,
    bearer: bool,
    pub language: String,
    slots: Semaphore,
}

impl Tmdb {
    /// `api` normally `https://api.themoviedb.org/3`, `images` `https://image.tmdb.org/t/p`.
    pub fn new(api: &str, images: &str, key: &str, language: &str) -> Tmdb {
        let key = key.trim().to_string();
        Tmdb {
            http: client(),
            api: api.trim_end_matches('/').to_string(),
            images: images.trim_end_matches('/').to_string(),
            bearer: key.starts_with("eyJ"), // v4 tokens are JWTs
            key,
            language: language.to_string(),
            slots: Semaphore::new(MAX_PARALLEL),
        }
    }

    /// The language without region, e.g. "de".
    pub fn lang2(&self) -> &str {
        self.language.get(..2).unwrap_or("en")
    }

    /// GET a path with query parameters; None on 404.
    pub async fn get(&self, path: &str, params: &[(&str, &str)]) -> MetaResult<Option<Value>> {
        let mut q: Vec<(&str, &str)> = params.to_vec();
        if !self.bearer {
            q.push(("api_key", &self.key));
        }
        for attempt in 0..4u64 {
            let mut req = self.http.get(format!("{}{path}", self.api)).query(&q).header("Accept", "application/json");
            if self.bearer {
                req = req.bearer_auth(&self.key);
            }
            let res = {
                let _slot = self.slots.acquire().await.map_err(|_| MetaError::new("error.tmdbBusy"))?;
                req.send().await
            };
            let res = match res {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!("TMDB {path}: {e}");
                    if attempt < 3 {
                        tokio::time::sleep(Duration::from_secs(2 * (attempt + 1))).await;
                        continue;
                    }
                    return Err(MetaError(Text::new("error.tmdbUnreachable").arg(e.to_string())));
                }
            };
            let status = res.status().as_u16();
            match status {
                200 => return res.json::<Value>().await.map(Some).map_err(|_| MetaError::new("error.tmdbBusy")),
                404 => return Ok(None),
                401 => return Err(MetaError::new("error.tmdbKeyRejected")),
                429 | 500..=599 => {
                    let wait =
                        res.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok()).unwrap_or(2u64);
                    tokio::time::sleep(Duration::from_secs(wait.clamp(1, 10) * (attempt + 1))).await;
                }
                _ => return Err(MetaError(Text::new("error.tmdbHttp").arg(i64::from(status)).arg(path.to_string()))),
            }
        }
        Err(MetaError::new("error.tmdbBusy"))
    }

    /// Downloads an image (`/abc.jpg`) in a size unless the target exists; false if TMDB has none.
    pub async fn download(&self, image: &str, size: &str, target: &Path) -> bool {
        let valid =
            image.len() > 1 && image.starts_with('/') && image[1..].bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b));
        if !valid {
            return false;
        }
        if target.is_file() {
            return true;
        }
        let Some(dir) = target.parent() else { return false };
        if tokio::fs::create_dir_all(dir).await.is_err() {
            return false;
        }
        let res = {
            let Ok(_slot) = self.slots.acquire().await else { return false };
            match self.http.get(format!("{}/{size}{image}", self.images)).send().await {
                Ok(r) if r.status().is_success() => r.bytes().await.ok(),
                _ => None,
            }
        };
        let Some(bytes) = res.filter(|b| !b.is_empty()) else { return false };
        let tmp = target.with_extension("part");
        tokio::fs::write(&tmp, &bytes).await.is_ok() && tokio::fs::rename(&tmp, target).await.is_ok()
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("HTTP client")
}

/// TVmaze: free show database without a key; used to find the IMDb or TVDB id of a show the TMDB
/// search misses (TMDB then resolves that id exactly).
pub struct TvMaze {
    http: reqwest::Client,
    api: String,
}

/// External ids of a show found on TVmaze.
#[derive(Debug, Default, Clone)]
pub struct ShowIds {
    pub name: String,
    pub year: i32,
    pub imdb: Option<String>,
    pub tvdb: Option<i64>,
}

impl TvMaze {
    pub fn new(api: &str) -> TvMaze {
        TvMaze { http: client(), api: api.trim_end_matches('/').to_string() }
    }

    /// Shows for a name with their external ids, best first.
    pub async fn search(&self, name: &str) -> Vec<ShowIds> {
        let res = self.http.get(format!("{}/search/shows", self.api)).query(&[("q", name)]).send().await;
        let Ok(res) = res else { return Vec::new() };
        let Ok(list) = res.json::<Value>().await else { return Vec::new() };
        list.as_array()
            .map(|a| {
                a.iter()
                    .map(|r| {
                        let s = &r["show"];
                        ShowIds {
                            name: s["name"].as_str().unwrap_or("").to_string(),
                            year: year_of(s["premiered"].as_str().unwrap_or("")),
                            imdb: s["externals"]["imdb"].as_str().map(str::to_string),
                            tvdb: s["externals"]["thetvdb"].as_i64(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The year of a date like "1999-03-31", 0 if none.
pub fn year_of(date: &str) -> i32 {
    date.get(..4).filter(|y| y.bytes().all(|b| b.is_ascii_digit())).and_then(|y| y.parse().ok()).unwrap_or(0)
}
