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

//! How to play a file in a given player: directly, or converted (container only, audio only, or
//! everything), depending on what the browser can decode, the chosen quality, the audio track
//! and subtitles that have to be burned in.

use super::{AudioStream, MediaFile};
use crate::i18n::Text;
use serde_json::Value;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Direct,
    Remux,
    Audio,
    Transcode,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Direct => "direct",
            Mode::Remux => "remux",
            Mode::Audio => "audio",
            Mode::Transcode => "transcode",
        }
    }
}

/// A quality step of the player menu: bitrate limit and maximum frame height (0 = none).
#[derive(Clone, Copy, Debug)]
pub struct Quality {
    pub id: &'static str,
    pub label: &'static str,
    pub video_bitrate: i64,
    pub max_height: i32,
}

pub const QUALITIES: [Quality; 7] = [
    Quality { id: "original", label: "Original", video_bitrate: 0, max_height: 0 },
    Quality { id: "20000", label: "20 Mbit/s 1080p", video_bitrate: 20_000_000, max_height: 1080 },
    Quality { id: "12000", label: "12 Mbit/s 1080p", video_bitrate: 12_000_000, max_height: 1080 },
    Quality { id: "8000", label: "8 Mbit/s 1080p", video_bitrate: 8_000_000, max_height: 1080 },
    Quality { id: "4000", label: "4 Mbit/s 720p", video_bitrate: 4_000_000, max_height: 720 },
    Quality { id: "2000", label: "2 Mbit/s 720p", video_bitrate: 2_000_000, max_height: 720 },
    Quality { id: "1000", label: "1 Mbit/s 480p", video_bitrate: 1_000_000, max_height: 480 },
];

/// Highest frame height the software encoder produces when the user picked "Original".
const SOFTWARE_MAX_HEIGHT: i32 = 1080;

pub fn quality(id: &str) -> Quality {
    QUALITIES.iter().find(|q| q.id == id).copied().unwrap_or(QUALITIES[0])
}

/// Whether `id` names a quality or `auto`.
pub fn known_quality(id: &str) -> bool {
    id == "auto" || QUALITIES.iter().any(|q| q.id == id)
}

/// The quality "automatic" stands for. In the home network the original; from outside the
/// best step that fits into the measured bandwidth (bits per second) with room to spare, the
/// original if the whole file fits, and 4 Mbit/s without a measurement.
pub fn automatic(home: bool, bandwidth: i64, file_bitrate: i64) -> Quality {
    if home {
        return QUALITIES[0];
    }
    if bandwidth <= 0 {
        return quality("4000");
    }
    // Two thirds of the measured speed: audio, container and the ups and downs of the line.
    let usable = bandwidth * 2 / 3;
    if file_bitrate > 0 && file_bitrate <= usable {
        return QUALITIES[0];
    }
    QUALITIES[1..].iter().find(|q| q.video_bitrate <= usable).copied().unwrap_or(QUALITIES[QUALITIES.len() - 1])
}

/// Whether a client address is in the home network: loopback, private and link-local ranges,
/// IPv6 unique local addresses.
pub fn home_network(ip: &str) -> bool {
    match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        Ok(std::net::IpAddr::V6(v6)) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_loopback() || v4.is_private() || v4.is_link_local();
            }
            v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// Decoder support reported by the browser.
#[derive(Debug, Default)]
pub struct Capabilities {
    pub containers: HashSet<String>,
    pub video: HashSet<String>,
    pub audio: HashSet<String>,
    pub mse_video: HashSet<String>,
    pub mse_audio: HashSet<String>,
    pub direct_play_allowed: bool,
    /// Whether 10-bit HEVC (Main10, up to 4K) decodes, separately for files and for MediaSource;
    /// None when the client does not say, which counts as yes (apps decode it in hardware).
    pub hevc_main10: Option<bool>,
    pub mse_hevc_main10: Option<bool>,
}

impl Capabilities {
    pub fn parse(m: &Value) -> Capabilities {
        let set = |k: &str| -> HashSet<String> {
            m[k].as_array()
                .map(|a| a.iter().map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()).to_lowercase()).collect())
                .unwrap_or_default()
        };
        Capabilities {
            containers: set("containers"),
            video: set("video"),
            audio: set("audio"),
            mse_video: set("mseVideo"),
            mse_audio: set("mseAudio"),
            direct_play_allowed: m["directPlay"].as_bool().unwrap_or(true),
            hevc_main10: m["hevcMain10"].as_bool(),
            mse_hevc_main10: m["mseHevcMain10"].as_bool(),
        }
    }
}

/// What happens to the sound when the server streams the file (not in direct play).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioOut {
    /// Copied as it is: the player decodes it (AAC, AC3, E-AC3, FLAC, Opus, MP3).
    Copy,
    /// Converted to AAC stereo.
    AacStereo,
    /// Multi-channel sound converted to Dolby Digital (5.1), for players that decode it.
    Ac3,
    /// Multi-channel sound converted to Dolby Digital Plus (5.1).
    Eac3,
}

impl AudioOut {
    pub fn label(self) -> &'static str {
        match self {
            AudioOut::Copy => "copy",
            AudioOut::AacStereo => "AAC",
            AudioOut::Ac3 => "AC3 5.1",
            AudioOut::Eac3 => "E-AC3 5.1",
        }
    }
}

/// Codecs a player may get unchanged inside HLS/fMP4, if its MediaSource says it can decode them.
const COPYABLE_AUDIO: [&str; 6] = ["aac", "mp3", "ac3", "eac3", "flac", "opus"];

/// The sound for a stream: copied if the player can decode it; multi-channel sound as E-AC3 or
/// AC3 when the player decodes that (5.1 stays 5.1); otherwise AAC stereo.
pub fn audio_out(a: Option<&AudioStream>, caps: &Capabilities) -> AudioOut {
    let Some(a) = a else { return AudioOut::Copy };
    let c = codec_kind(&a.codec);
    if caps.mse_audio.contains(&c) && COPYABLE_AUDIO.contains(&c.as_str()) {
        AudioOut::Copy
    } else if a.channels > 2 && caps.mse_audio.contains("eac3") {
        AudioOut::Eac3
    } else if a.channels > 2 && caps.mse_audio.contains("ac3") {
        AudioOut::Ac3
    } else {
        AudioOut::AacStereo
    }
}

/// The decision.
#[derive(Debug)]
pub struct Plan {
    pub mode: Mode,
    /// Position in the file's audio streams, -1 if the file has no audio.
    pub audio_index: i32,
    /// The sound when streaming (ignored for direct play).
    pub audio_out: AudioOut,
    /// Stream index of a bitmap subtitle to burn in, or -1.
    pub burn_subtitle: i32,
    /// Target bitrate and height when transcoding (height 0 = keep).
    pub video_bitrate: i64,
    pub max_height: i32,
    /// A text subtitle (WebVTT) to burn into the picture, see [`burn_text`].
    pub burn_text: Option<String>,
    /// Explanation for the player info.
    pub reason: Text,
}

impl Plan {
    fn new(mode: Mode, audio_index: i32, reason: Text) -> Plan {
        Plan { mode, audio_index, audio_out: AudioOut::Copy, burn_subtitle: -1, video_bitrate: 0, max_height: 0, burn_text: None, reason }
    }
}

/// ffprobe container names as the browser check names them.
pub fn container_kind(f: &MediaFile) -> String {
    let (fmt, ext) = (f.format_name.as_str(), f.container.as_str());
    if fmt.contains("mp4") || fmt.contains("mov") || matches!(ext, "mp4" | "m4v" | "mov") {
        "mp4".into()
    } else if ext == "webm" {
        "webm".into()
    } else if fmt.contains("matroska") || ext == "mkv" {
        "mkv".into()
    } else {
        ext.to_string()
    }
}

/// ffprobe codec names as the browser check names them.
pub fn codec_kind(codec: &str) -> String {
    match codec.to_lowercase().as_str() {
        "h265" => "hevc".into(),
        "avc" => "h264".into(),
        "eac3" | "e-ac-3" => "eac3".into(),
        "ac-3" => "ac3".into(),
        c => c.to_string(),
    }
}

/// 10-bit or 4:2:2/4:4:4 video, which browsers usually cannot decode for H.264.
pub fn high_bit_depth_or_chroma(f: &MediaFile) -> bool {
    let p = f.pixel_format.as_str();
    p.contains("10") || p.contains("12") || p.contains("422") || p.contains("444")
}

pub fn plan(f: &MediaFile, caps: &Capabilities, q: Quality, requested_audio: i32, subtitle: Option<&str>, transcoder: bool) -> Plan {
    let audio_index = choose_audio(f, requested_audio);
    let audio = usize::try_from(audio_index).ok().and_then(|i| f.audio_streams.get(i));
    let burn = subtitle
        .and_then(|s| s.strip_prefix('e'))
        .and_then(|n| n.parse::<i32>().ok())
        .and_then(|n| f.subtitle_streams.iter().find(|s| s.stream_index == n))
        .filter(|s| !s.text_based())
        .map_or(-1, |s| s.stream_index);
    let video = codec_kind(&f.video_codec);
    let hbd = high_bit_depth_or_chroma(f);
    let bitrate_ok = q.video_bitrate == 0
        || (f.bitrate > 0 && f.bitrate as f64 <= q.video_bitrate as f64 * 1.1) && (q.max_height == 0 || f.height <= q.max_height);
    // Browsers that decode HEVC often do so only for 8 bits (Main); 10-bit files (HDR, most 4K)
    // then fail in the decoder although "hevc" is supported.
    let hevc10 = video == "hevc" && hbd;
    let video_decodable = caps.video.contains(&video) && !(video == "h264" && hbd) && !(hevc10 && caps.hevc_main10 == Some(false));
    let audio_decodable = audio.is_none_or(|a| caps.audio.contains(&codec_kind(&a.codec)));
    let first_audio = audio_index <= 0;
    let container_ok = caps.containers.contains(&container_kind(f));
    if !transcoder {
        return Plan::new(Mode::Direct, audio_index, Text::new("playback.transcoderOff"));
    }
    if burn < 0 && bitrate_ok && video_decodable && audio_decodable && first_audio && container_ok && caps.direct_play_allowed {
        return Plan::new(Mode::Direct, audio_index, Text::new("playback.direct"));
    }
    // Video can be copied into HLS if the browser's MediaSource accepts it and nothing forces a re-encode.
    let video_copyable = burn < 0
        && bitrate_ok
        && caps.mse_video.contains(&video)
        && !(video == "h264" && hbd)
        && !(hevc10 && caps.mse_hevc_main10 == Some(false));
    let audio_out = audio_out(audio, caps);
    if video_copyable {
        if audio_out == AudioOut::Copy {
            return Plan::new(Mode::Remux, audio_index, Text::new("playback.remux").arg(container_kind(f)));
        }
        let codec = audio.map(|a| a.codec.to_uppercase()).unwrap_or_default();
        let mut p = Plan::new(Mode::Audio, audio_index, Text::new("playback.audio").arg(codec).arg(audio_out.label()));
        p.audio_out = audio_out;
        return p;
    }
    let (target, height) = transcode_targets(f, q);
    let reason = if burn >= 0 {
        Text::new("playback.burnSubtitles")
    } else if !video_decodable || (bitrate_ok && !video_copyable) {
        let extra = if hbd { format!(" {}", f.pixel_format) } else { String::new() };
        Text::new("playback.video").arg(format!("{}{extra}", f.video_codec.to_uppercase()))
    } else {
        Text::new("playback.quality")
    };
    Plan {
        mode: Mode::Transcode,
        audio_index,
        audio_out,
        burn_subtitle: burn,
        video_bitrate: target,
        max_height: height,
        burn_text: None,
        reason,
    }
}

/// Estimated bits per second of the stream a plan sends: the file's for direct play and copied
/// video, the target of a conversion plus its sound.
pub fn stream_bitrate(f: &MediaFile, plan: &Plan) -> i64 {
    let audio = match plan.audio_out {
        AudioOut::Copy => 0,
        AudioOut::AacStereo => 192_000,
        AudioOut::Ac3 | AudioOut::Eac3 => 640_000,
    };
    match plan.mode {
        Mode::Transcode => plan.video_bitrate + audio,
        _ => f.bitrate,
    }
}

/// Bitrate and frame height of a conversion at quality `q`.
fn transcode_targets(f: &MediaFile, q: Quality) -> (i64, i32) {
    let source = if f.bitrate > 0 { f.bitrate } else { 8_000_000 };
    let target = if q.video_bitrate == 0 { source.min(20_000_000) } else { q.video_bitrate }.max(500_000);
    (target, if q.max_height == 0 { SOFTWARE_MAX_HEIGHT } else { q.max_height })
}

/// Turns a plan into a conversion that burns the text subtitle `vtt` into the picture (for
/// players that cannot show subtitles themselves, or styled the way the file has them).
pub fn burn_text(f: &MediaFile, mut plan: Plan, q: Quality, vtt: String) -> Plan {
    if plan.mode != Mode::Transcode {
        (plan.video_bitrate, plan.max_height) = transcode_targets(f, q);
        plan.mode = Mode::Transcode;
    }
    plan.burn_subtitle = -1;
    plan.burn_text = Some(vtt);
    plan.reason = Text::new("playback.burnSubtitles");
    plan
}

/// Direct play for a native player that decodes every format itself (mpv in FreeX Desktop).
pub fn external(f: &MediaFile, requested_audio: i32) -> Plan {
    Plan::new(Mode::Direct, choose_audio(f, requested_audio), Text::new("playback.external"))
}

/// The requested track if valid, otherwise the one flagged as default, otherwise the first.
pub fn choose_audio(f: &MediaFile, requested: i32) -> i32 {
    if f.audio_streams.is_empty() {
        return -1;
    }
    if requested >= 0 && (requested as usize) < f.audio_streams.len() {
        return requested;
    }
    f.audio_streams.iter().position(|a| a.is_default).unwrap_or(0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::SubtitleStream;
    use serde_json::json;

    fn file() -> MediaFile {
        MediaFile {
            container: "mkv".into(),
            format_name: "matroska,webm".into(),
            video_codec: "h264".into(),
            pixel_format: "yuv420p".into(),
            height: 1080,
            bitrate: 6_000_000,
            audio_streams: vec![
                AudioStream { codec: "aac".into(), ..AudioStream::default() },
                AudioStream { codec: "ac3".into(), is_default: true, ..AudioStream::default() },
            ],
            subtitle_streams: vec![SubtitleStream { stream_index: 4, codec: "hdmv_pgs_subtitle".into(), ..SubtitleStream::default() }],
            ..MediaFile::default()
        }
    }

    fn chrome() -> Capabilities {
        Capabilities::parse(&json!({"containers": ["mp4", "webm", "mkv"], "video": ["h264", "vp9"], "audio": ["aac", "opus"],
                                     "mseVideo": ["h264"], "mseAudio": ["aac"]}))
    }

    #[test]
    fn automatic_quality() {
        assert_eq!(automatic(true, 0, 30_000_000).id, "original");
        assert_eq!(automatic(false, 0, 30_000_000).id, "4000");
        assert_eq!(automatic(false, 100_000_000, 30_000_000).id, "original");
        assert_eq!(automatic(false, 10_000_000, 30_000_000).id, "4000");
        assert_eq!(automatic(false, 20_000_000, 30_000_000).id, "12000");
        assert_eq!(automatic(false, 500_000, 30_000_000).id, "1000");
        assert!(home_network("192.168.1.20") && home_network("10.0.0.1") && home_network("127.0.0.1"));
        assert!(home_network("fd12:3456::1") && home_network("::ffff:192.168.0.5") && home_network("::1"));
        assert!(!home_network("8.8.8.8") && !home_network("2a01:4f8::1") && !home_network("?"));
    }

    #[test]
    fn decides() {
        let f = file();
        let c = chrome();
        assert_eq!(plan(&f, &c, quality("original"), 0, None, false).mode, Mode::Direct);
        assert_eq!(plan(&f, &c, quality("original"), 0, None, true).mode, Mode::Direct);
        // The default track is AC3: the audio has to be converted.
        assert_eq!(plan(&f, &c, quality("original"), -1, None, true).mode, Mode::Audio);
        assert_eq!(plan(&f, &c, quality("original"), 0, Some("e4"), true).mode, Mode::Transcode);
        let p = plan(&f, &c, quality("2000"), 0, None, true);
        assert_eq!((p.mode, p.video_bitrate, p.max_height), (Mode::Transcode, 2_000_000, 720));
        let mut hevc = file();
        hevc.video_codec = "hevc".into();
        assert_eq!(plan(&hevc, &c, quality("original"), 0, None, true).reason.key, "playback.video");
        assert_eq!(choose_audio(&f, 7), 1);
        assert_eq!(external(&f, 0).mode, Mode::Direct);
    }

    #[test]
    fn ten_bit_hevc() {
        let mut f = file();
        f.video_codec = "hevc".into();
        f.pixel_format = "yuv420p10le".into();
        f.audio_streams = vec![AudioStream { codec: "eac3".into(), ..AudioStream::default() }];
        let edge = |main10: bool| {
            Capabilities::parse(&json!({"containers": ["mp4"], "video": ["h264", "hevc"], "audio": ["aac"], "mseVideo": ["h264", "hevc"],
                                         "mseAudio": ["aac"], "hevcMain10": main10, "mseHevcMain10": main10}))
        };
        // Main10 decodes: the video is copied, only the sound converted.
        assert_eq!(plan(&f, &edge(true), quality("original"), 0, None, true).mode, Mode::Audio);
        // Only 8-bit HEVC: the whole title is converted.
        assert_eq!(plan(&f, &edge(false), quality("original"), 0, None, true).mode, Mode::Transcode);
        // Apps that do not say keep copying.
        let app = Capabilities::parse(
            &json!({"containers": ["mp4"], "video": ["hevc"], "audio": ["aac"], "mseVideo": ["hevc"], "mseAudio": ["aac"]}),
        );
        assert_eq!(plan(&f, &app, quality("original"), 0, None, true).mode, Mode::Audio);
        // 8-bit HEVC is not affected.
        f.pixel_format = "yuv420p".into();
        assert_eq!(plan(&f, &edge(false), quality("original"), 0, None, true).mode, Mode::Audio);
    }

    #[test]
    fn keeps_surround_sound() {
        let a = |codec: &str, channels| AudioStream { codec: codec.into(), channels, ..AudioStream::default() };
        let chrome = chrome();
        let edge = Capabilities::parse(&json!({"mseVideo": ["h264"], "mseAudio": ["aac", "ac3", "eac3"]}));
        let ac3_only = Capabilities::parse(&json!({"mseVideo": ["h264"], "mseAudio": ["aac", "ac3"]}));
        // Copied whenever the player can decode it, also when the video is converted.
        assert_eq!(audio_out(Some(&a("eac3", 6)), &edge), AudioOut::Copy);
        assert_eq!(audio_out(Some(&a("aac", 6)), &chrome), AudioOut::Copy);
        // DTS 5.1: E-AC3 or AC3 if the player has it, otherwise AAC stereo; stereo stays AAC.
        assert_eq!(audio_out(Some(&a("dts", 6)), &edge), AudioOut::Eac3);
        assert_eq!(audio_out(Some(&a("truehd", 8)), &ac3_only), AudioOut::Ac3);
        assert_eq!(audio_out(Some(&a("dts", 6)), &chrome), AudioOut::AacStereo);
        assert_eq!(audio_out(Some(&a("dts", 2)), &edge), AudioOut::AacStereo);
        // A converted video keeps the AC3 track for a player that decodes AC3.
        let mut f = file();
        f.video_codec = "hevc".into();
        let p = plan(&f, &edge, quality("original"), 1, None, true);
        assert_eq!((p.mode, p.audio_out), (Mode::Transcode, AudioOut::Copy));
    }
}
