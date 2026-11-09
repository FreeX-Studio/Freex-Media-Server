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

//! Audio fingerprints for finding stretches that sound the same in two files (the intro or the
//! end credits of a season). The sound is reduced to 8 kHz mono, cut into frames, and each
//! frame becomes a 32-bit hash of how the energy in 33 frequency bands changes against the frame
//! before (the classic "Philips" fingerprint). Two frames count as equal when few bits differ.

use crate::media::probe::command;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;

const SAMPLE_RATE: usize = 8000;
const FFT_SIZE: usize = 2048;
const HOP: usize = 512;
/// Length of one frame in seconds.
pub const FRAME_SECONDS: f64 = HOP as f64 / SAMPLE_RATE as f64;
const BANDS: usize = 33;
const MIN_HZ: f64 = 300.0;
const MAX_HZ: f64 = 3000.0;
/// Frames quieter than this (mean square of 16-bit samples) carry no information.
const SILENCE: f64 = 2000.0;

pub struct Fingerprint {
    /// Fingerprint values per frame.
    pub hashes: Vec<u32>,
    /// False for silent frames, which must never count as a match.
    pub valid: Vec<bool>,
    /// Position of frame 0 in the file, in seconds.
    pub start_seconds: f64,
}

impl Fingerprint {
    pub fn frames(&self) -> usize {
        self.hashes.len()
    }

    /// Decodes `seconds` of the first audio track from `start` with ffmpeg and fingerprints it.
    pub async fn of(ffmpeg: &str, file: &Path, start: f64, seconds: f64) -> Option<Fingerprint> {
        let mut c = command(ffmpeg);
        c.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-ss", &format!("{start:.3}"), "-t", &format!("{seconds:.3}"), "-i"])
            .arg(file)
            .args(["-map", "0:a:0", "-ac", "1", "-ar", &SAMPLE_RATE.to_string(), "-f", "s16le", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = c.spawn().ok()?;
        let mut out = child.stdout.take()?;
        let max = (seconds * SAMPLE_RATE as f64 * 2.0) as usize + 65_536;
        let read = async {
            let mut pcm = Vec::with_capacity(max.min(1 << 22));
            let mut buf = vec![0u8; 65_536];
            loop {
                let n = out.read(&mut buf).await.ok()?;
                if n == 0 {
                    break;
                }
                let room = max.saturating_sub(pcm.len());
                pcm.extend_from_slice(&buf[..n.min(room)]);
                if pcm.len() >= max {
                    break;
                }
            }
            Some(pcm)
        };
        let pcm = match tokio::time::timeout(Duration::from_secs(120), read).await {
            Ok(Some(p)) => p,
            _ => {
                let _ = child.kill().await;
                return None;
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
        let samples: Vec<i16> = pcm.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        tokio::task::spawn_blocking(move || compute(&samples, start)).await.ok()
    }
}

pub fn compute(samples: &[i16], start_seconds: f64) -> Fingerprint {
    let frames = if samples.len() < FFT_SIZE { 0 } else { (samples.len() - FFT_SIZE) / HOP + 1 };
    let mut hashes = Vec::with_capacity(frames);
    let mut valid = Vec::with_capacity(frames);
    let edges = band_edges();
    let window: Vec<f64> =
        (0..FFT_SIZE).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (FFT_SIZE - 1) as f64).cos()).collect();
    let mut re = vec![0.0; FFT_SIZE];
    let mut im = vec![0.0; FFT_SIZE];
    let mut prev = [0.0f64; BANDS];
    let mut cur = [0.0f64; BANDS];
    for f in 0..frames {
        let off = f * HOP;
        let mut power = 0.0;
        for i in 0..FFT_SIZE {
            let s = f64::from(samples[off + i]);
            power += s * s;
            re[i] = s * window[i];
            im[i] = 0.0;
        }
        power /= FFT_SIZE as f64;
        fft(&mut re, &mut im);
        for b in 0..BANDS {
            cur[b] = (edges[b]..edges[b + 1]).map(|k| re[k] * re[k] + im[k] * im[k]).sum();
        }
        let mut h = 0u32;
        if f > 0 {
            for b in 0..BANDS - 1 {
                let d = (cur[b] - cur[b + 1]) - (prev[b] - prev[b + 1]);
                if d > 0.0 {
                    h |= 1 << b;
                }
            }
        }
        hashes.push(h);
        valid.push(f > 0 && power > SILENCE);
        std::mem::swap(&mut prev, &mut cur);
    }
    Fingerprint { hashes, valid, start_seconds }
}

fn band_edges() -> [usize; BANDS + 1] {
    let bin_hz = SAMPLE_RATE as f64 / FFT_SIZE as f64;
    let mut edges = [0usize; BANDS + 1];
    for (b, e) in edges.iter_mut().enumerate() {
        let hz = MIN_HZ * (MAX_HZ / MIN_HZ).powf(b as f64 / BANDS as f64);
        *e = (hz / bin_hz).round() as usize;
    }
    for b in 1..=BANDS {
        if edges[b] <= edges[b - 1] {
            edges[b] = edges[b - 1] + 1;
        }
    }
    edges
}

/// In-place iterative radix-2 FFT.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for i in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (i + k, i + k + len / 2);
                let xr = re[b] * cr - im[b] * ci;
                let xi = re[b] * ci + im[b] * cr;
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
                let t = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = t;
            }
        }
        len <<= 1;
    }
}

/// A stretch present in both fingerprints, in frames of each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start_a: usize,
    pub end_a: usize,
    pub start_b: usize,
    pub end_b: usize,
}

impl Match {
    pub fn length(&self) -> usize {
        self.end_a - self.start_a
    }
}

/// Finds the longest stretch that sounds the same in `a` and `b`, at any time shift. Frames
/// count as equal with at most `max_bit_errors` differing bits; short disturbances of up to
/// `max_gap` frames inside a stretch are tolerated.
pub fn longest_common(a: &Fingerprint, b: &Fingerprint, min_frames: usize, max_bit_errors: u32, max_gap: usize) -> Option<Match> {
    let (na, nb) = (a.frames() as i64, b.frames() as i64);
    let min = min_frames as i64;
    let mut best: Option<Match> = None;
    let mut shift = -(nb - min);
    while shift <= na - min {
        let i_start = shift.max(0);
        let i_end = na.min(nb + shift);
        let mut run_start: i64 = -1;
        let mut last_match: i64 = -1;
        let mut matches = 0usize;
        let mut i = i_start;
        while i < i_end {
            let j = (i - shift) as usize;
            let iu = i as usize;
            let eq = a.valid[iu] && b.valid[j] && (a.hashes[iu] ^ b.hashes[j]).count_ones() <= max_bit_errors;
            if eq {
                if run_start < 0 {
                    run_start = i;
                    matches = 0;
                }
                last_match = i;
                matches += 1;
            } else if run_start >= 0 && i - last_match > max_gap as i64 {
                best = better(best, run_start, last_match + 1, shift, matches);
                run_start = -1;
            }
            i += 1;
        }
        if run_start >= 0 {
            best = better(best, run_start, last_match + 1, shift, matches);
        }
        shift += 1;
    }
    best.filter(|m| m.length() >= min_frames)
}

fn better(best: Option<Match>, start: i64, end: i64, shift: i64, matches: usize) -> Option<Match> {
    // Require that most frames of the stretch really match, not just a few scattered ones.
    if (matches as f64) < (end - start) as f64 * 0.6 {
        return best;
    }
    let length = (end - start) as usize;
    if best.is_none_or(|b| length > b.length()) {
        return Some(Match {
            start_a: start as usize,
            end_a: end as usize,
            start_b: (start - shift) as usize,
            end_b: (end - shift) as usize,
        });
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic "melody": a few tones in a row.
    fn tone_sequence(seconds: f64, seed: u32) -> Vec<i16> {
        let n = (seconds * SAMPLE_RATE as f64) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / SAMPLE_RATE as f64;
                let step = (t * 2.0) as u32 + seed; // a new tone every half second
                let hz = 400.0 + f64::from((step * 7919) % 1200);
                ((2.0 * std::f64::consts::PI * hz * t).sin() * 8000.0 + (2.0 * std::f64::consts::PI * hz * 1.5 * t).sin() * 3000.0) as i16
            })
            .collect()
    }

    fn noise(seconds: f64, seed: u64) -> Vec<i16> {
        let n = (seconds * SAMPLE_RATE as f64) as usize;
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x % 16000) as i16 - 8000
            })
            .collect()
    }

    #[test]
    fn finds_the_shared_intro() {
        // Episode A: 10 s noise, 30 s intro, noise. Episode B: 25 s noise, the same intro, noise.
        let intro = tone_sequence(30.0, 3);
        let mut a = noise(10.0, 1);
        a.extend(&intro);
        a.extend(noise(20.0, 2));
        let mut b = noise(25.0, 4);
        b.extend(&intro);
        b.extend(noise(5.0, 5));
        let (fa, fb) = (compute(&a, 0.0), compute(&b, 0.0));
        let min_frames = (15.0 / FRAME_SECONDS).ceil() as usize;
        let m = longest_common(&fa, &fb, min_frames, 10, 24).expect("match");
        let start_a = m.start_a as f64 * FRAME_SECONDS;
        let start_b = m.start_b as f64 * FRAME_SECONDS;
        let length = m.length() as f64 * FRAME_SECONDS;
        assert!((start_a - 10.0).abs() < 1.5, "start a {start_a}");
        assert!((start_b - 25.0).abs() < 1.5, "start b {start_b}");
        assert!((length - 30.0).abs() < 3.0, "length {length}");
        // Unrelated sound gives no match.
        let fc = compute(&noise(40.0, 9), 0.0);
        assert!(longest_common(&fa, &fc, min_frames, 10, 24).is_none());
    }
}
