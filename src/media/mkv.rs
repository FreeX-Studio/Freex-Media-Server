//! Keyframe times from the index (Cues) of a Matroska or WebM file.
//!
//! Muxers such as mkvmerge and ffmpeg write a cue point for the keyframes of the video track,
//! usually at the end of the file. Reading them needs a handful of small reads (the headers,
//! the seek head and the cues), where asking ffprobe to seek through the file needs one seek
//! per two seconds of video: on a hard disk or a network share that is the difference between
//! milliseconds and half a minute.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const INFO: u32 = 0x1549_A966;
const TIMESTAMP_SCALE: u32 = 0x2A_D7B1;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_TYPE: u32 = 0x83;
const CLUSTER: u32 = 0x1F43_B675;
const CUES: u32 = 0x1C53_BB6B;
const CUE_POINT: u32 = 0xBB;
const CUE_TIME: u32 = 0xB3;
const CUE_TRACK_POSITIONS: u32 = 0xB7;
const CUE_TRACK: u32 = 0xF7;

/// Largest header element read into memory (seek head, info, tracks).
const MAX_HEADER: u64 = 4 << 20;
/// Largest index read into memory; a film with a cue every second has about 100 KB.
const MAX_CUES: u64 = 64 << 20;
/// Top-level elements looked at before giving up on finding the headers.
const MAX_ELEMENTS: usize = 64;

/// Keyframe times in ms of the video track that is stream `stream_index` (ffprobe's
/// numbering; a negative index means the first video track), sorted, from the file's
/// timestamp zero. None when the file is no Matroska file or has no usable index.
pub fn keyframes(path: &Path, stream_index: i32) -> Option<Vec<i64>> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut r = Reader { f: &mut f, len };
    // EBML header, then the segment.
    let (id, size) = r.header()?;
    if id != EBML {
        return None;
    }
    r.skip(size)?;
    let (id, size) = r.header()?;
    if id != SEGMENT {
        return None;
    }
    let segment = r.pos()?;
    let segment_end = size.map_or(len, |s| (segment + s).min(len));

    let mut scale: u64 = 1_000_000;
    let mut tracks: Option<Vec<(u64, u64)>> = None;
    let mut cues_at: Option<u64> = None;
    let mut info_at: Option<u64> = None;
    let mut tracks_at: Option<u64> = None;
    // The top-level elements up to the first cluster: seek head, info, tracks (and attachments,
    // chapters, tags, which are skipped). The cues are usually after the clusters.
    for _ in 0..MAX_ELEMENTS {
        let start = r.pos()?;
        if start >= segment_end {
            break;
        }
        let Some((id, size)) = r.header() else { break };
        let Some(size) = size else { break };
        match id {
            SEEK_HEAD => {
                for (target, pos) in seek_entries(&r.body(size, MAX_HEADER)?) {
                    match target {
                        CUES => cues_at = cues_at.or(Some(segment + pos)),
                        INFO => info_at = info_at.or(Some(segment + pos)),
                        TRACKS => tracks_at = tracks_at.or(Some(segment + pos)),
                        _ => {}
                    }
                }
            }
            INFO => scale = timestamp_scale(&r.body(size, MAX_HEADER)?),
            TRACKS => tracks = Some(track_entries(&r.body(size, MAX_HEADER)?)),
            CUES => {
                cues_at = Some(start);
                break;
            }
            CLUSTER => break,
            _ => r.skip(Some(size))?,
        }
    }
    // Info and tracks the seek head points at, when they were not in front of the clusters.
    if tracks.is_none() {
        let (id, size) = r.at(tracks_at?)?;
        tracks = (id == TRACKS).then(|| r.body(size?, MAX_HEADER).map(|b| track_entries(&b))).flatten();
    }
    if let (Some(at), true) = (info_at, scale == 1_000_000)
        && let Some((INFO, Some(size))) = r.at(at)
    {
        scale = timestamp_scale(&r.body(size, MAX_HEADER)?);
    }
    let track = video_track(tracks.as_deref()?, stream_index)?;
    let (id, size) = r.at(cues_at?)?;
    if id != CUES {
        return None;
    }
    cue_times(&r.body(size?, MAX_CUES)?, track, scale)
}

struct Reader<'a> {
    f: &'a mut File,
    len: u64,
}

impl Reader<'_> {
    fn pos(&mut self) -> Option<u64> {
        self.f.stream_position().ok()
    }

    fn byte(&mut self) -> Option<u8> {
        let mut b = [0u8; 1];
        self.f.read_exact(&mut b).ok()?;
        Some(b[0])
    }

    /// An element id with its length marker kept, as the ids above are written.
    fn id(&mut self) -> Option<u32> {
        let first = self.byte()?;
        let n = first.leading_zeros() as usize + 1;
        if n > 4 {
            return None;
        }
        let mut id = u32::from(first);
        for _ in 1..n {
            id = (id << 8) | u32::from(self.byte()?);
        }
        Some(id)
    }

    /// An element size; None inside for "unknown size" (live streams, unfinished files).
    fn size(&mut self) -> Option<Option<u64>> {
        let first = self.byte()?;
        let n = first.leading_zeros() as usize + 1;
        if n > 8 {
            return None;
        }
        let mut v = u64::from(first) & (0xFF >> n);
        let mut all_ones = v == (0xFF >> n);
        for _ in 1..n {
            let b = self.byte()?;
            all_ones &= b == 0xFF;
            v = (v << 8) | u64::from(b);
        }
        Some(if all_ones { None } else { Some(v) })
    }

    fn header(&mut self) -> Option<(u32, Option<u64>)> {
        Some((self.id()?, self.size()?))
    }

    fn at(&mut self, pos: u64) -> Option<(u32, Option<u64>)> {
        if pos >= self.len {
            return None;
        }
        self.f.seek(SeekFrom::Start(pos)).ok()?;
        self.header()
    }

    fn skip(&mut self, size: Option<u64>) -> Option<()> {
        let size = size?;
        let to = self.pos()?.checked_add(size)?;
        (to <= self.len).then_some(())?;
        self.f.seek(SeekFrom::Start(to)).ok().map(|_| ())
    }

    fn body(&mut self, size: u64, max: u64) -> Option<Vec<u8>> {
        if size > max || self.pos()?.checked_add(size)? > self.len {
            return None;
        }
        let mut buf = vec![0u8; usize::try_from(size).ok()?];
        self.f.read_exact(&mut buf).ok()?;
        Some(buf)
    }
}

/// The child elements of a body: (id, content).
fn children(mut b: &[u8]) -> Vec<(u32, &[u8])> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let Some((id, n)) = vint(b, true) else { break };
        let Some((size, m)) = vint(&b[n..], false) else { break };
        let start = n + m;
        let Some(end) = usize::try_from(size).ok().and_then(|s| start.checked_add(s)).filter(|&e| e <= b.len()) else { break };
        out.push((id as u32, &b[start..end]));
        b = &b[end..];
    }
    out
}

/// A variable-length integer of a buffer and its length; ids keep their length marker.
fn vint(b: &[u8], keep_marker: bool) -> Option<(u64, usize)> {
    let first = *b.first()?;
    let n = first.leading_zeros() as usize + 1;
    if n > 8 || b.len() < n {
        return None;
    }
    let mut v = if keep_marker { u64::from(first) } else { u64::from(first) & (0xFF >> n) };
    for &x in &b[1..n] {
        v = (v << 8) | u64::from(x);
    }
    Some((v, n))
}

fn uint(b: &[u8]) -> u64 {
    b.iter().take(8).fold(0, |v, &x| (v << 8) | u64::from(x))
}

fn seek_entries(b: &[u8]) -> Vec<(u32, u64)> {
    children(b)
        .into_iter()
        .filter(|(id, _)| *id == SEEK)
        .filter_map(|(_, seek)| {
            let c = children(seek);
            let target = c.iter().find(|(id, _)| *id == SEEK_ID).map(|(_, v)| uint(v) as u32)?;
            let pos = c.iter().find(|(id, _)| *id == SEEK_POSITION).map(|(_, v)| uint(v))?;
            Some((target, pos))
        })
        .collect()
}

fn timestamp_scale(b: &[u8]) -> u64 {
    children(b).into_iter().find(|(id, _)| *id == TIMESTAMP_SCALE).map(|(_, v)| uint(v)).filter(|&s| s > 0).unwrap_or(1_000_000)
}

/// (track number, track type) in the order of the file, which is ffprobe's stream order.
fn track_entries(b: &[u8]) -> Vec<(u64, u64)> {
    children(b)
        .into_iter()
        .filter(|(id, _)| *id == TRACK_ENTRY)
        .map(|(_, e)| {
            let c = children(e);
            let get = |want: u32| c.iter().find(|(id, _)| *id == want).map(|(_, v)| uint(v)).unwrap_or(0);
            (get(TRACK_NUMBER), get(TRACK_TYPE))
        })
        .collect()
}

/// The track number of the video stream: the entry at the stream's index if that is a video
/// track, otherwise the first video track.
fn video_track(tracks: &[(u64, u64)], stream_index: i32) -> Option<u64> {
    let video = |t: &&(u64, u64)| t.1 == 1 && t.0 > 0;
    usize::try_from(stream_index).ok().and_then(|i| tracks.get(i)).filter(video).or_else(|| tracks.iter().find(video)).map(|t| t.0)
}

fn cue_times(b: &[u8], track: u64, scale: u64) -> Option<Vec<i64>> {
    let mut out: Vec<i64> = children(b)
        .into_iter()
        .filter(|(id, _)| *id == CUE_POINT)
        .filter_map(|(_, p)| {
            let c = children(p);
            let time = c.iter().find(|(id, _)| *id == CUE_TIME).map(|(_, v)| uint(v))?;
            let ours = c
                .iter()
                .filter(|(id, _)| *id == CUE_TRACK_POSITIONS)
                .any(|(_, pos)| children(pos).iter().any(|(id, v)| *id == CUE_TRACK && uint(v) == track));
            ours.then(|| i64::try_from(u128::from(time) * u128::from(scale) / 1_000_000).ok()).flatten()
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints() {
        assert_eq!(vint(&[0x1A, 0x45, 0xDF, 0xA3], true), Some((0x1A45_DFA3, 4)));
        assert_eq!(vint(&[0x81], false), Some((1, 1)));
        assert_eq!(vint(&[0x40, 0x02], false), Some((2, 2)));
        assert_eq!(vint(&[0x00], false), None);
    }

    #[test]
    fn picks_the_video_track() {
        let tracks = [(1, 1), (2, 2), (3, 17)];
        assert_eq!(video_track(&tracks, 0), Some(1));
        assert_eq!(video_track(&tracks, 1), Some(1));
        assert_eq!(video_track(&tracks, -1), Some(1));
        assert_eq!(video_track(&[(2, 2)], 0), None);
    }

    #[test]
    fn not_matroska() {
        let p = std::env::temp_dir().join(format!("freex-mkv-test-{}.bin", std::process::id()));
        std::fs::write(&p, b"\x00\x00\x00\x18ftypmp42").unwrap();
        assert_eq!(keyframes(&p, 0), None);
        let _ = std::fs::remove_file(&p);
    }
}
