//! Song IDs: one permanent, very unique ID per song, the same on every device, never based on names.
//!
//! * A song with an audio file: computed from the audio itself (SHA-256 of the audio data, with the tag
//!   blocks left out), so renaming, retagging or new artwork never changes it, and the same file gives the
//!   same ID on any device without them talking to each other. Covers and songs that share a name have
//!   different audio, so different IDs.
//! * A song without audio yet (written with the songwriter, stems only): random, made once and kept in its
//!   song.json so it travels with the song.
//! * A saved Apple Music song: from Apple's catalog number, so it matches on every device.
//!
//! IDs are 160 bits, written `LT-` + 32 characters in groups of four (Crockford base32: no I, L, O or U).

use anyhow::Result;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 20 bytes → "LT-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX".
fn format(bytes: &[u8; 20]) -> String {
    let mut bits = 0u64;
    let mut n = 0;
    let mut out = String::with_capacity(42);
    out.push_str("LT");
    let mut chars = 0;
    for b in bytes {
        bits = (bits << 8) | *b as u64;
        n += 8;
        while n >= 5 {
            n -= 5;
            if chars % 4 == 0 {
                out.push('-');
            }
            out.push(ALPHABET[((bits >> n) & 31) as usize] as char);
            chars += 1;
        }
    }
    out
}

fn from_digest(h: Sha256) -> String {
    let d = h.finalize();
    let mut b = [0u8; 20];
    b.copy_from_slice(&d[..20]);
    format(&b)
}

/// A new random ID (songs without audio).
pub fn random() -> String {
    let mut b = [0u8; 32];
    // the OS's randomness, via a fresh file read where available; otherwise time + process mixed in a hash
    let ok = File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_ok();
    let mut h = Sha256::new();
    h.update(b"lt-song-random-v1:");
    h.update(b);
    if !ok {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        h.update(t.as_nanos().to_le_bytes());
        h.update(std::process::id().to_le_bytes());
        h.update(format!("{:p}", &b).as_bytes());
    }
    from_digest(h)
}

/// The ID of a song from a streaming service (e.g. "apple", catalog id).
pub fn for_service(service: &str, id: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"lt-song-service-v1:");
    h.update(service.as_bytes());
    h.update(b":");
    h.update(id.as_bytes());
    from_digest(h)
}

/// The same song again (an exact copy of the audio already in the library): still its own ID.
pub fn variant(id: &str, n: u32) -> String {
    let mut h = Sha256::new();
    h.update(b"lt-song-copy-v1:");
    h.update(id.as_bytes());
    h.update(n.to_le_bytes());
    from_digest(h)
}

/// The ID of an audio file, from its audio data only.
pub fn for_file(path: &Path) -> Result<String> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let ranges = audio_ranges(&mut f, len).unwrap_or_else(|_| vec![(0, len)]);
    let mut h = Sha256::new();
    h.update(b"lt-song-audio-v1:");
    let mut buf = vec![0u8; 1 << 16];
    for (start, end) in ranges {
        f.seek(SeekFrom::Start(start))?;
        let mut left = end.saturating_sub(start);
        while left > 0 {
            let want = left.min(buf.len() as u64) as usize;
            let n = f.read(&mut buf[..want])?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            left -= n as u64;
        }
    }
    Ok(from_digest(h))
}

fn read_at(f: &mut File, at: u64, n: usize) -> std::io::Result<Vec<u8>> {
    f.seek(SeekFrom::Start(at))?;
    let mut b = vec![0u8; n];
    let got = f.read(&mut b)?;
    b.truncate(got);
    Ok(b)
}

fn be32(b: &[u8]) -> u64 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64
}
fn le32(b: &[u8]) -> u64 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64
}

/// The byte ranges that hold the audio (tags left out). Unknown formats: the whole file.
fn audio_ranges(f: &mut File, len: u64) -> std::io::Result<Vec<(u64, u64)>> {
    let head = read_at(f, 0, 12)?;
    if head.len() < 12 {
        return Ok(vec![(0, len)]);
    }
    // MP4 / M4A: the audio is in the 'mdat' boxes
    if &head[4..8] == b"ftyp" {
        let mut out = vec![];
        let mut at = 0u64;
        while at + 8 <= len {
            let h = read_at(f, at, 16)?;
            if h.len() < 8 {
                break;
            }
            let (mut size, mut hdr) = (be32(&h), 8u64);
            if size == 1 && h.len() >= 16 {
                size = u64::from_be_bytes(h[8..16].try_into().unwrap());
                hdr = 16;
            } else if size == 0 {
                size = len - at;
            }
            if size < hdr {
                break;
            }
            if &h[4..8] == b"mdat" {
                out.push((at + hdr, (at + size).min(len)));
            }
            at += size;
        }
        return Ok(if out.is_empty() { vec![(0, len)] } else { out });
    }
    // WAV (RIFF 'data') and AIFF (FORM 'SSND')
    if (&head[0..4] == b"RIFF" && &head[8..12] == b"WAVE") || (&head[0..4] == b"FORM" && (&head[8..12] == b"AIFF" || &head[8..12] == b"AIFC")) {
        let riff = &head[0..4] == b"RIFF";
        let want: &[u8] = if riff { b"data" } else { b"SSND" };
        let mut at = 12u64;
        while at + 8 <= len {
            let h = read_at(f, at, 8)?;
            let size = if riff { le32(&h[4..8]) } else { be32(&h[4..8]) };
            if &h[0..4] == want {
                return Ok(vec![(at + 8, (at + 8 + size).min(len))]);
            }
            at += 8 + size + (size & 1);
        }
        return Ok(vec![(0, len)]);
    }
    // Ogg (Vorbis, Opus): page bodies after the header packets (which hold the tags)
    if &head[0..4] == b"OggS" {
        return ogg_ranges(f, len);
    }
    // MP3, FLAC and anything else: skip ID3v2 at the start, then FLAC's metadata blocks; trim tag blocks at the end
    let mut start = 0u64;
    let id3 = read_at(f, 0, 10)?;
    if id3.len() == 10 && &id3[0..3] == b"ID3" {
        let size = ((id3[6] as u64 & 0x7f) << 21) | ((id3[7] as u64 & 0x7f) << 14) | ((id3[8] as u64 & 0x7f) << 7) | (id3[9] as u64 & 0x7f);
        start = 10 + size + if id3[5] & 0x10 != 0 { 10 } else { 0 };
    }
    if read_at(f, start, 4)? == b"fLaC" {
        let mut at = start + 4;
        loop {
            let h = read_at(f, at, 4)?;
            if h.len() < 4 {
                break;
            }
            let size = ((h[1] as u64) << 16) | ((h[2] as u64) << 8) | h[3] as u64;
            at += 4 + size;
            if h[0] & 0x80 != 0 {
                break;
            }
        }
        start = at;
    }
    let mut end = len;
    loop {
        if end >= start + 128 && read_at(f, end - 128, 3)? == b"TAG" {
            end -= 128;
            continue;
        }
        if end >= start + 32 {
            let ape = read_at(f, end - 32, 32)?;
            if ape.len() == 32 && &ape[0..8] == b"APETAGEX" {
                let size = le32(&ape[12..16]);
                let has_header = ape[23] & 0x80 != 0;
                end = end.saturating_sub(size + if has_header { 32 } else { 0 }).max(start);
                continue;
            }
        }
        if end >= start + 15 && read_at(f, end - 9, 9)? == b"LYRICS200" {
            let n = read_at(f, end - 15, 6)?;
            if let Ok(size) = String::from_utf8_lossy(&n).trim().parse::<u64>() {
                end = end.saturating_sub(size + 15).max(start);
                continue;
            }
        }
        break;
    }
    Ok(vec![(start.min(len), end)])
}

fn ogg_ranges(f: &mut File, len: u64) -> std::io::Result<Vec<(u64, u64)>> {
    let mut out = vec![];
    let mut at = 0u64;
    let mut headers_left: Option<u32> = None; // packets still to skip once the codec is known
    while at + 27 <= len {
        let h = read_at(f, at, 27)?;
        if h.len() < 27 || &h[0..4] != b"OggS" {
            break;
        }
        let nseg = h[26] as usize;
        let segs = read_at(f, at + 27, nseg)?;
        let body = at + 27 + nseg as u64;
        if headers_left.is_none() {
            let first = read_at(f, body, 8)?;
            headers_left = Some(if first.starts_with(b"\x01vorbis") { 3 } else if first.starts_with(b"OpusHead") { 2 } else { 0 });
        }
        let mut pos = body;
        for &lace in &segs {
            let seg_end = pos + lace as u64;
            if headers_left == Some(0) {
                out.push((pos, seg_end.min(len)));
            } else if lace < 255 {
                headers_left = headers_left.map(|n| n - 1); // a packet ended
            }
            pos = seg_end;
        }
        at = pos;
    }
    // merge touching ranges
    let mut merged: Vec<(u64, u64)> = vec![];
    for r in out {
        match merged.last_mut() {
            Some(last) if last.1 == r.0 => last.1 = r.1,
            _ => merged.push(r),
        }
    }
    Ok(if merged.is_empty() { vec![(0, len)] } else { merged })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id3(size: usize) -> Vec<u8> {
        let mut v = b"ID3\x04\x00\x00".to_vec();
        v.extend([(size >> 21 & 0x7f) as u8, (size >> 14 & 0x7f) as u8, (size >> 7 & 0x7f) as u8, (size & 0x7f) as u8]);
        v.extend(std::iter::repeat_n(b'x', size));
        v
    }

    #[test]
    fn retagging_keeps_the_id_and_different_audio_changes_it() {
        let d = tempfile::tempdir().unwrap();
        let audio: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut a = id3(40);
        a.extend(&audio);
        let mut b = id3(400); // new tags, more artwork: bigger tag block
        b.extend(&audio);
        b.extend(b"TAG");
        b.extend([0u8; 125]); // and an old ID3v1 tag at the end
        let mut c = id3(40);
        c.extend(audio.iter().map(|x| x ^ 1)); // different audio, same tags
        for (n, v) in [("a.mp3", &a), ("b.mp3", &b), ("c.mp3", &c)] {
            std::fs::write(d.path().join(n), v).unwrap();
        }
        let (ia, ib, ic) = (for_file(&d.path().join("a.mp3")).unwrap(), for_file(&d.path().join("b.mp3")).unwrap(), for_file(&d.path().join("c.mp3")).unwrap());
        assert_eq!(ia, ib, "retagged: same song");
        assert_ne!(ia, ic, "different audio: different song");
        assert_eq!(ia.len(), 42);
        assert!(ia.starts_with("LT-") && ia.split('-').count() == 9, "{ia}");
    }

    #[test]
    fn mp4_and_wav_hash_only_their_audio() {
        let d = tempfile::tempdir().unwrap();
        let box_ = |kind: &[u8], body: &[u8]| {
            let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
            v.extend(kind);
            v.extend(body);
            v
        };
        let audio = vec![9u8; 3000];
        let mk = |meta: &[u8]| [box_(b"ftyp", b"M4A mp42"), box_(b"moov", meta), box_(b"mdat", &audio)].concat();
        std::fs::write(d.path().join("a.m4a"), mk(b"title one")).unwrap();
        std::fs::write(d.path().join("b.m4a"), mk(b"a much longer title and new artwork")).unwrap();
        assert_eq!(for_file(&d.path().join("a.m4a")).unwrap(), for_file(&d.path().join("b.m4a")).unwrap());
        let wav = |extra: &[u8]| {
            let mut v = b"RIFF\0\0\0\0WAVE".to_vec();
            v.extend(b"LIST");
            v.extend((extra.len() as u32).to_le_bytes());
            v.extend(extra);
            v.extend(b"data");
            v.extend((audio.len() as u32).to_le_bytes());
            v.extend(&audio);
            v
        };
        std::fs::write(d.path().join("a.wav"), wav(b"ab")).unwrap();
        std::fs::write(d.path().join("b.wav"), wav(b"other tags")).unwrap();
        assert_eq!(for_file(&d.path().join("a.wav")).unwrap(), for_file(&d.path().join("b.wav")).unwrap());
    }

    #[test]
    fn random_and_service_ids() {
        assert_ne!(random(), random());
        assert_eq!(for_service("apple", "1440833098"), for_service("apple", "1440833098"));
        assert_ne!(for_service("apple", "1"), for_service("apple", "2"));
        assert_ne!(variant("LT-X", 2), variant("LT-X", 3));
    }
}
