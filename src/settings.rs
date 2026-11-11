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

//! The settings an admin can change in the web interface, and how they are written into
//! `config.toml`. The API keeps the field names of FreeX Web (`server.bindAddress`); each maps to
//! a key of the TOML file (`server.bind`). Writes go through `toml_edit`, so comments and the
//! layout of the file survive.

use crate::config::Config;
use crate::i18n::Text;
use serde_json::{Value, json};
use std::path::Path;
use toml_edit::{DocumentMut, Item, Table, value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Number,
    Bool,
    Choice,
    Secret,
    ReadOnly,
}

/// One setting of the web interface.
pub struct Field {
    /// Name in the API, as FreeX Web knows it.
    pub key: &'static str,
    /// `[section]` and key in `config.toml`.
    pub section: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    pub min: i64,
    pub max: i64,
    pub choices: &'static [&'static str],
    pub max_length: usize,
    /// Catalog key describing the expected format, checked by [`format_ok`].
    pub format: &'static str,
}

const fn text(key: &'static str, section: &'static str, name: &'static str, max_length: usize, format: &'static str) -> Field {
    Field { key, section, name, kind: Kind::Text, min: 0, max: 0, choices: &[], max_length, format }
}

const fn number(key: &'static str, section: &'static str, name: &'static str, min: i64, max: i64) -> Field {
    Field { key, section, name, kind: Kind::Number, min, max, choices: &[], max_length: 0, format: "" }
}

const fn of(key: &'static str, section: &'static str, name: &'static str, kind: Kind) -> Field {
    Field {
        key,
        section,
        name,
        kind,
        min: 0,
        max: 0,
        choices: &[],
        max_length: if matches!(kind, Kind::Secret) { 512 } else { 0 },
        format: "",
    }
}

const fn choice(key: &'static str, section: &'static str, name: &'static str, choices: &'static [&'static str]) -> Field {
    Field { key, section, name, kind: Kind::Choice, min: 0, max: 0, choices, max_length: 0, format: "" }
}

/// All settings in the order of the web interface.
pub const FIELDS: &[Field] = &[
    text("server.name", "server", "name", 60, ""),
    text("server.bindAddress", "server", "bind", 100, "setting.format.address"),
    number("server.port", "server", "port", 1, 65535),
    text("server.tlsCertificate", "server", "tls_certificate", 500, ""),
    text("server.tlsKey", "server", "tls_key", 500, ""),
    of("server.trustProxy", "server", "trust_proxy", Kind::Bool),
    of("data.directory", "data", "directory", Kind::ReadOnly),
    of("data.cacheDirectory", "data", "cache_directory", Kind::ReadOnly),
    of("ffmpeg.ffprobe", "ffmpeg", "ffprobe", Kind::ReadOnly),
    of("ffmpeg.ffmpeg", "ffmpeg", "ffmpeg", Kind::ReadOnly),
    number("security.sessionDays", "security", "session_days", 1, 365),
    number("security.minPasswordLength", "security", "min_password_length", 8, 128),
    of("tmdb.apiKey", "tmdb", "api_key", Kind::Secret),
    text("tmdb.language", "tmdb", "language", 10, "setting.format.language"),
    text("tmdb.region", "tmdb", "region", 2, "setting.format.region"),
    of("tmdb.tvmaze", "tmdb", "tvmaze", Kind::Bool),
    of("markers.enabled", "markers", "enabled", Kind::Bool),
    of("previews.enabled", "previews", "enabled", Kind::Bool),
    of("transcoder.enabled", "transcoder", "enabled", Kind::Bool),
    text("transcoder.directory", "transcoder", "directory", 500, ""),
    number("transcoder.maxSessions", "transcoder", "max_sessions", 1, 32),
    choice("transcoder.hardwareAcceleration", "transcoder", "hardware_acceleration", &["none", "nvenc", "qsv", "vaapi"]),
    text("transcoder.vaapiDevice", "transcoder", "vaapi_device", 100, "setting.format.vaapiDevice"),
    choice("transcoder.preset", "transcoder", "preset", &["ultrafast", "superfast", "veryfast", "faster", "fast", "medium"]),
    of("dlna.enabled", "dlna", "enabled", Kind::Bool),
    text("dlna.user", "dlna", "user", 64, ""),
    text("dlna.name", "dlna", "name", 60, ""),
    number("dlna.port", "dlna", "port", 1, 65535),
    of("jellyfin.enabled", "jellyfin", "enabled", Kind::Bool),
    of("maintenance.enabled", "maintenance", "enabled", Kind::Bool),
    text("maintenance.start", "maintenance", "start", 5, "setting.format.time"),
    number("maintenance.backups", "maintenance", "backups", 0, 60),
    number("scanner.intervalMinutes", "scanner", "interval_minutes", 0, 10080),
    number("scanner.threads", "scanner", "threads", 1, 32),
];

pub fn field(key: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|f| f.key == key)
}

impl Field {
    pub fn to_public(&self) -> Value {
        let mut m = json!({"key": self.key, "type": match self.kind {
            Kind::Text => "text",
            Kind::Number => "number",
            Kind::Bool => "bool",
            Kind::Choice => "choice",
            Kind::Secret => "secret",
            Kind::ReadOnly => "readonly",
        }});
        if self.kind == Kind::Number {
            m["min"] = json!(self.min);
            m["max"] = json!(self.max);
        }
        if self.kind == Kind::Choice {
            m["choices"] = json!(self.choices);
        }
        m
    }

    /// The value of this setting in a loaded configuration, as text.
    pub fn current(&self, c: &Config) -> String {
        match self.key {
            "server.name" => c.server.name.clone(),
            "server.bindAddress" => c.server.bind.clone(),
            "server.port" => c.server.port.to_string(),
            "server.tlsCertificate" => c.server.tls_certificate.clone(),
            "server.tlsKey" => c.server.tls_key.clone(),
            "server.trustProxy" => c.server.trust_proxy.to_string(),
            "data.directory" => c.data.directory.to_string_lossy().into_owned(),
            "data.cacheDirectory" => c.data.cache_directory.to_string_lossy().into_owned(),
            "ffmpeg.ffprobe" => c.ffmpeg.ffprobe.clone(),
            "ffmpeg.ffmpeg" => c.ffmpeg.ffmpeg.clone(),
            "security.sessionDays" => c.security.session_days.to_string(),
            "security.minPasswordLength" => c.security.min_password_length.to_string(),
            "tmdb.apiKey" => c.tmdb.api_key.clone(),
            "tmdb.language" => c.tmdb.language.clone(),
            "tmdb.region" => c.tmdb.region.clone(),
            "tmdb.tvmaze" => c.tmdb.tvmaze.to_string(),
            "markers.enabled" => c.markers.enabled.to_string(),
            "previews.enabled" => c.previews.enabled.to_string(),
            "transcoder.enabled" => c.transcoder.enabled.to_string(),
            "transcoder.directory" => c.transcoder.directory.to_string_lossy().into_owned(),
            "transcoder.maxSessions" => c.transcoder.max_sessions.to_string(),
            "transcoder.hardwareAcceleration" => c.transcoder.hardware_acceleration.clone(),
            "transcoder.vaapiDevice" => c.transcoder.vaapi_device.clone(),
            "transcoder.preset" => c.transcoder.preset.clone(),
            "scanner.intervalMinutes" => c.scanner.interval_minutes.to_string(),
            "dlna.enabled" => c.dlna.enabled.to_string(),
            "dlna.user" => c.dlna.user.clone(),
            "dlna.name" => c.dlna.name.clone(),
            "dlna.port" => c.dlna.port.to_string(),
            "jellyfin.enabled" => c.jellyfin.enabled.to_string(),
            "maintenance.enabled" => c.maintenance.enabled.to_string(),
            "maintenance.start" => c.maintenance.start.clone(),
            "maintenance.backups" => c.maintenance.backups.to_string(),
            "scanner.threads" => c.scanner.threads.to_string(),
            _ => String::new(),
        }
    }

    /// The value for the API: numbers and switches typed, the rest as text.
    pub fn typed(&self, text: &str) -> Value {
        match self.kind {
            Kind::Bool => json!(text.trim() == "true"),
            Kind::Number => text.trim().parse::<i64>().map(Value::from).unwrap_or_else(|_| json!(text)),
            _ => json!(text),
        }
    }
}

/// Checks one submitted value and returns it as text in its normalized form.
pub fn validate(f: &Field, v: &Value) -> Result<String, Text> {
    let key = f.key.to_string();
    match f.kind {
        Kind::ReadOnly => Err(Text::new("setting.readOnly").arg(key)),
        Kind::Bool => v.as_bool().map(|b| b.to_string()).ok_or_else(|| Text::new("setting.expectBool").arg(key)),
        Kind::Number => {
            let n = match v {
                Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)),
                Value::String(s) => s.trim().parse().ok(),
                _ => None,
            }
            .ok_or_else(|| Text::new("setting.expectInteger").arg(key.clone()))?;
            if n < f.min || n > f.max {
                return Err(Text::new("setting.range").arg(key).arg(f.min).arg(f.max));
            }
            Ok(n.to_string())
        }
        Kind::Choice => {
            let s = v.as_str().unwrap_or("");
            if f.choices.contains(&s) { Ok(s.to_string()) } else { Err(Text::new("setting.choices").arg(key).arg(f.choices.join(", "))) }
        }
        Kind::Text | Kind::Secret => {
            let raw = v.as_str().ok_or_else(|| Text::new("setting.expectText").arg(key.clone()))?;
            let s = if f.kind == Kind::Text { raw.trim() } else { raw };
            if s.chars().any(char::is_control) {
                return Err(Text::new("setting.controlCharacters").arg(key));
            }
            if f.max_length > 0 && s.chars().count() > f.max_length {
                return Err(Text::new("setting.maxLength").arg(key).arg(f.max_length));
            }
            if !f.format.is_empty() && !s.is_empty() && !format_ok(f.format, s) {
                return Err(Text::new("setting.format").arg(key).arg(Text::new(f.format)));
            }
            if s.is_empty() {
                match f.key {
                    "server.name" => return Err(Text::new("setting.emptyServerName")),
                    "server.bindAddress" => return Err(Text::new("setting.emptyAddress")),
                    "transcoder.directory" => return Err(Text::new("setting.emptyTranscodeDirectory")),
                    _ => {}
                }
            }
            Ok(s.to_string())
        }
    }
}

fn format_ok(format: &str, s: &str) -> bool {
    match format {
        "setting.format.address" => s.chars().all(|c| c.is_ascii_alphanumeric() || ".:%-".contains(c)),
        "setting.format.language" => {
            let b = s.as_bytes();
            (b.len() == 2 && b.iter().all(u8::is_ascii_lowercase))
                || (b.len() == 5
                    && b[0].is_ascii_lowercase()
                    && b[1].is_ascii_lowercase()
                    && b[2] == b'-'
                    && b[3].is_ascii_uppercase()
                    && b[4].is_ascii_uppercase())
        }
        "setting.format.region" => s.len() == 2 && s.bytes().all(|b| b.is_ascii_uppercase()),
        "setting.format.time" => {
            crate::config::MaintenanceConfig { start: s.to_string(), ..Default::default() }.start_time().is_some() && s.len() == 5
        }
        "setting.format.vaapiDevice" => {
            s.strip_prefix("/dev/dri/").is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_alphanumeric()))
        }
        _ => true,
    }
}

/// Reads the file, sets the changed values (field → normalized text) and replaces the file
/// atomically, keeping comments and layout. The new copy keeps the old file's permissions.
pub fn write(path: &Path, changes: &[(&Field, String)]) -> Result<(), String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    for (f, v) in changes {
        let section = doc.entry(f.section).or_insert(Item::Table(Table::new()));
        let Some(table) = section.as_table_mut() else { return Err(format!("{} is not a table", f.section)) };
        let new_value = match f.kind {
            Kind::Bool => value(v == "true"),
            Kind::Number => value(v.parse::<i64>().map_err(|e| e.to_string())?),
            _ => value(v.as_str()),
        };
        match table.get_mut(f.name).and_then(Item::as_value_mut) {
            // An existing line keeps its spacing and trailing comment.
            Some(existing) => {
                let decor = existing.decor().clone();
                *existing = new_value.into_value().expect("scalar");
                *existing.decor_mut() = decor;
            }
            None => {
                table.insert(f.name, new_value);
            }
        }
    }
    let updated = doc.to_string();
    // The file is parsed once more, so a value the server would reject never lands in it.
    toml::from_str::<Config>(&updated).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &updated).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path).map(|m| m.permissions().mode()).unwrap_or(0o600);
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    }
    // The old file is kept: if the server does not start with the new one, it goes back to it.
    let _ = std::fs::copy(path, previous_file(path));
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot replace {}: {e}", path.display()))
}

/// The copy of the configuration before the last change from the settings page.
pub fn previous_file(path: &Path) -> std::path::PathBuf {
    path.with_extension("toml.previous")
}

/// Puts the previous configuration back after a failed restart and keeps the failed one as
/// `config.toml.failed`; false if there is no previous file.
pub fn restore_previous(path: &Path) -> bool {
    let previous = previous_file(path);
    if !previous.is_file() {
        return false;
    }
    let _ = std::fs::rename(path, path.with_extension("toml.failed"));
    std::fs::rename(&previous, path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates() {
        let port = field("server.port").unwrap();
        assert_eq!(validate(port, &json!(8080)).unwrap(), "8080");
        assert_eq!(validate(port, &json!("70000")).unwrap_err().key, "setting.range");
        let lang = field("tmdb.language").unwrap();
        assert_eq!(validate(lang, &json!(" de-DE ")).unwrap(), "de-DE");
        assert_eq!(validate(lang, &json!("deutsch")).unwrap_err().key, "setting.format");
        let hw = field("transcoder.hardwareAcceleration").unwrap();
        assert_eq!(validate(hw, &json!("cuda")).unwrap_err().key, "setting.choices");
        assert_eq!(validate(field("data.directory").unwrap(), &json!("/x")).unwrap_err().key, "setting.readOnly");
        assert_eq!(validate(field("server.name").unwrap(), &json!("  ")).unwrap_err().key, "setting.emptyServerName");
    }

    #[test]
    fn writes_keeping_comments() {
        let dir = std::env::temp_dir().join(format!("freex-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        std::fs::write(&p, "# top comment\n[server]\n# the port\nport = 32500 # inline\n").unwrap();
        write(
            &p,
            &[
                (field("server.port").unwrap(), "9000".into()),
                (field("tmdb.apiKey").unwrap(), "k\"ey".into()),
                (field("markers.enabled").unwrap(), "false".into()),
            ],
        )
        .unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("# top comment") && t.contains("# the port") && t.contains("port = 9000 # inline"));
        assert!(t.contains("[tmdb]") && t.contains("api_key = "));
        assert!(t.contains("[markers]") && t.contains("enabled = false"));
        let c: Config = toml::from_str(&t).unwrap();
        assert_eq!((c.server.port, c.tmdb.api_key.as_str(), c.markers.enabled), (9000, "k\"ey", false));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
