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

//! `config.toml`: read once at start. Missing keys take their defaults; on the first start the
//! file is written with all keys and comments. Relative paths count from the file's folder.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The file written on the first start; also documents every key.
pub const DEFAULT_FILE: &str = r#"# FreeX Media Server configuration.
# Changes take effect after a restart of the server.

[server]
# Name shown in the web interface and to apps
name = "FreeX"
# Address and port of the web interface; "0.0.0.0" listens on all network interfaces
bind = "0.0.0.0"
port = 32500
# Set to true when a reverse proxy (nginx, Caddy, ...) is in front of the server: client
# addresses and HTTPS are then taken from its X-Forwarded-* headers
trust_proxy = false
# HTTPS: certificate (with its chain) and private key as PEM files, relative to this file.
# Both empty = plain HTTP, e.g. behind a reverse proxy that does TLS
tls_certificate = ""
tls_key = ""

[data]
# Database and everything the server cannot recreate; relative to this file
directory = "data"
# Files the server can recreate at any time (thumbnails, conversions); safe to delete while
# the server is stopped
cache_directory = "cache"

[ffmpeg]
# The ffprobe and ffmpeg programs: names on the PATH or full paths. The server also works
# without them, but without media information, generated thumbnails and conversion
ffprobe = "ffprobe"
ffmpeg = "ffmpeg"

[tmdb]
# API key (v3) or read access token (v4) from https://www.themoviedb.org/settings/api (free).
# Empty: only local .nfo files and artwork are used
api_key = ""
# Language of titles and descriptions, and country for age ratings
language = "de-DE"
region = "DE"
# Ask TVmaze (free, no key) for shows the TMDB search does not find
tvmaze = true

[markers]
# Detect intros and end credits of TV episodes ("Skip intro") by comparing the sound of the
# episodes of a season; needs ffmpeg
enabled = true

[previews]
# Small frames above the seek bar while scrubbing, made with ffmpeg in the background after
# scans (one sprite sheet per title in the cache directory)
enabled = true

[transcoder]
# Convert files the browser cannot play (codec, container, bitrate) with ffmpeg
enabled = true
# Where temporary segments are written: relative to the cache directory or an absolute path,
# e.g. on a fast SSD. The server only ever deletes its own session folders there
directory = "transcode"
# Simultaneous conversions; more requests get a "busy" message
max_sessions = 3
# Hardware encoding: "none", "nvenc" (NVIDIA), "qsv" (Intel Quick Sync) or "vaapi" (Intel/AMD on Linux)
hardware_acceleration = "none"
vaapi_device = "/dev/dri/renderD128"
# x264 speed/quality trade-off for software encoding ("ultrafast" ... "medium")
preset = "veryfast"

[maintenance]
# A nightly window: database backup, cache clean-up, rescan of all libraries (with metadata,
# intro detection and seek previews). Time of day in the server's time zone
enabled = true
start = "03:00"
# Database backups kept in <data>/backups (0 = none)
backups = 7

[library]
# Admins may delete movies and episodes from disk on the item page. The server needs write access
# to the media folders for that (the Debian service: a ReadWritePaths drop-in)
allow_delete = false

[scanner]
# Automatic rescan of all libraries, in minutes (0 = only manual scans)
interval_minutes = 30
# Number of files probed with ffprobe at the same time
threads = 4

[security]
# A sign-in stays valid this many days without use
session_days = 30
# Minimum length of new passwords
min_password_length = 10
"#;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: Server,
    pub data: Data,
    pub security: Security,
    pub ffmpeg: Ffmpeg,
    pub scanner: ScannerConfig,
    pub transcoder: Transcoder,
    pub tmdb: TmdbConfig,
    pub markers: MarkersConfig,
    pub previews: PreviewsConfig,
    pub maintenance: MaintenanceConfig,
    pub library: LibraryConfig,
    /// Folder of the configuration file; relative paths are resolved against it.
    #[serde(skip)]
    pub base_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Server {
    pub name: String,
    pub bind: String,
    pub port: u16,
    pub trust_proxy: bool,
    pub tls_certificate: String,
    pub tls_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Data {
    pub directory: PathBuf,
    pub cache_directory: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ffmpeg {
    pub ffprobe: String,
    pub ffmpeg: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TmdbConfig {
    pub api_key: String,
    pub language: String,
    pub region: String,
    pub tvmaze: bool,
}

impl Default for TmdbConfig {
    fn default() -> Self {
        TmdbConfig { api_key: String::new(), language: "de-DE".into(), region: "DE".into(), tvmaze: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MarkersConfig {
    pub enabled: bool,
}

impl Default for MarkersConfig {
    fn default() -> Self {
        MarkersConfig { enabled: true }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LibraryConfig {
    pub allow_delete: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MaintenanceConfig {
    pub enabled: bool,
    /// "HH:MM", local time.
    pub start: String,
    pub backups: u32,
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        MaintenanceConfig { enabled: true, start: "03:00".into(), backups: 7 }
    }
}

impl MaintenanceConfig {
    /// Hour and minute of `start`; None if it is not a time of day.
    pub fn start_time(&self) -> Option<(u32, u32)> {
        let (h, m) = self.start.trim().split_once(':')?;
        let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
        (h < 24 && m < 60 && self.start.trim().len() <= 5).then_some((h, m))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PreviewsConfig {
    pub enabled: bool,
}

impl Default for PreviewsConfig {
    fn default() -> Self {
        PreviewsConfig { enabled: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Transcoder {
    pub enabled: bool,
    pub directory: PathBuf,
    pub max_sessions: u32,
    pub hardware_acceleration: String,
    pub vaapi_device: String,
    pub preset: String,
}

impl Default for Transcoder {
    fn default() -> Self {
        Transcoder {
            enabled: true,
            directory: PathBuf::from("transcode"),
            max_sessions: 3,
            hardware_acceleration: "none".into(),
            vaapi_device: "/dev/dri/renderD128".into(),
            preset: "veryfast".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScannerConfig {
    pub interval_minutes: u32,
    pub threads: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Security {
    pub session_days: u32,
    pub min_password_length: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            server: Server::default(),
            data: Data::default(),
            security: Security::default(),
            ffmpeg: Ffmpeg::default(),
            scanner: ScannerConfig::default(),
            transcoder: Transcoder::default(),
            tmdb: TmdbConfig::default(),
            markers: MarkersConfig::default(),
            previews: PreviewsConfig::default(),
            maintenance: MaintenanceConfig::default(),
            library: LibraryConfig::default(),
            base_dir: PathBuf::from("."),
        }
    }
}

impl Default for Server {
    fn default() -> Self {
        Server {
            name: "FreeX".into(),
            bind: "0.0.0.0".into(),
            port: 32500,
            trust_proxy: false,
            tls_certificate: String::new(),
            tls_key: String::new(),
        }
    }
}

impl Default for Data {
    fn default() -> Self {
        Data { directory: PathBuf::from("data"), cache_directory: PathBuf::from("cache") }
    }
}

impl Default for Ffmpeg {
    fn default() -> Self {
        Ffmpeg { ffprobe: "ffprobe".into(), ffmpeg: "ffmpeg".into() }
    }
}

impl Default for ScannerConfig {
    fn default() -> Self {
        ScannerConfig { interval_minutes: 30, threads: 4 }
    }
}

impl Default for Security {
    fn default() -> Self {
        Security { session_days: 30, min_password_length: 10 }
    }
}

impl Config {
    /// Reads the file, writing the default one first if it does not exist.
    pub fn load(path: &Path) -> Result<Config, String> {
        if !path.exists() {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            }
            std::fs::write(path, DEFAULT_FILE).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            tracing::info!("Wrote the default configuration to {}", path.display());
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut config: Config = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        config.base_dir = path.parent().filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if self.server.name.trim().is_empty() || self.server.name.chars().count() > 64 {
            return Err("server.name must have 1 to 64 characters".into());
        }
        if self.server.port == 0 {
            return Err("server.port must be between 1 and 65535".into());
        }
        if self.server.tls_certificate.trim().is_empty() != self.server.tls_key.trim().is_empty() {
            return Err("server.tls_certificate and server.tls_key must be set together".into());
        }
        if !(1..=3650).contains(&self.security.session_days) {
            return Err("security.session_days must be between 1 and 3650".into());
        }
        if !(8..=128).contains(&self.security.min_password_length) {
            return Err("security.min_password_length must be between 8 and 128".into());
        }
        if self.scanner.interval_minutes > 7 * 24 * 60 {
            return Err("scanner.interval_minutes must be between 0 and 10080".into());
        }
        if !(1..=32).contains(&self.scanner.threads) {
            return Err("scanner.threads must be between 1 and 32".into());
        }
        if !(1..=32).contains(&self.transcoder.max_sessions) {
            return Err("transcoder.max_sessions must be between 1 and 32".into());
        }
        if !["none", "nvenc", "qsv", "vaapi"].contains(&self.transcoder.hardware_acceleration.as_str()) {
            return Err("transcoder.hardware_acceleration must be \"none\", \"nvenc\", \"qsv\" or \"vaapi\"".into());
        }
        let lang = &self.tmdb.language;
        let lang_ok = lang.len() == 5 && lang.as_bytes()[2] == b'-' && lang.is_ascii();
        if !lang_ok {
            return Err("tmdb.language must look like \"de-DE\"".into());
        }
        if self.tmdb.region.len() != 2 || !self.tmdb.region.is_ascii() {
            return Err("tmdb.region must be a country code like \"DE\"".into());
        }
        let presets = ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"];
        if !presets.contains(&self.transcoder.preset.as_str()) {
            return Err(format!("transcoder.preset must be one of {}", presets.join(", ")));
        }
        Ok(())
    }

    /// A configured path, relative to the configuration file's folder unless absolute.
    pub fn resolve(&self, p: &Path) -> PathBuf {
        if p.is_absolute() { p.to_path_buf() } else { self.base_dir.join(p) }
    }

    pub fn data_dir(&self) -> PathBuf {
        self.resolve(&self.data.directory)
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.resolve(&self.data.cache_directory)
    }

    /// Certificate and key files for HTTPS, or None for plain HTTP.
    pub fn tls_files(&self) -> Option<(PathBuf, PathBuf)> {
        let (c, k) = (self.server.tls_certificate.trim(), self.server.tls_key.trim());
        (!c.is_empty() && !k.is_empty()).then(|| (self.resolve(Path::new(c)), self.resolve(Path::new(k))))
    }

    /// The folder for conversion segments, relative to the cache directory unless absolute.
    pub fn transcode_dir(&self) -> PathBuf {
        let d = &self.transcoder.directory;
        if d.is_absolute() { d.clone() } else { self.cache_dir().join(d) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_matches_defaults() {
        let c: Config = toml::from_str(DEFAULT_FILE).unwrap();
        assert_eq!(c.server.port, Server::default().port);
        assert_eq!(c.security.session_days, Security::default().session_days);
        assert_eq!(c.scanner.interval_minutes, ScannerConfig::default().interval_minutes);
        assert_eq!(c.ffmpeg.ffprobe, Ffmpeg::default().ffprobe);
        c.validate().unwrap();
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(toml::from_str::<Config>("[server]\nprot = 1\n").is_err());
    }
}
