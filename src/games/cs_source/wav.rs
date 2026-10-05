//! Source sound files (RIFF WAVE), per specs/cs_source/sounds.md "Mapping
//! to our engine": PCM (8-bit unsigned, 16-bit signed) and Microsoft ADPCM,
//! mono or stereo, with an optional loop start from a `cue ` or `smpl`
//! chunk. Decoded to interleaved 16-bit samples.

use crate::map::MapSoundClip;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn i16_at(b: &[u8], at: usize) -> Option<i16> {
    Some(i16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

/// Decode a WAV file.
pub fn decode(bytes: &[u8]) -> Result<MapSoundClip, String> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err("not a RIFF WAVE file".into());
    }
    let (mut fmt, mut data, mut loop_start) = (None, None, None);
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(bytes, at + 4).unwrap_or(0) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        match id {
            b"fmt " => fmt = Some(body),
            b"data" => data = Some(body),
            // One cue point: its sample offset (at byte 20 of the point).
            b"cue " if u32_at(body, 0).unwrap_or(0) > 0 => loop_start = u32_at(body, 4 + 20).map(|v| v as usize),
            // A sampler loop: its start (loop record at 36, start at +8).
            b"smpl" if u32_at(body, 28).unwrap_or(0) > 0 && loop_start.is_none() => {
                loop_start = u32_at(body, 36 + 8).map(|v| v as usize)
            }
            _ => {}
        }
        // Chunks are padded to even sizes.
        at += 8 + size + (size & 1);
    }
    let fmt = fmt.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    let tag = u16_at(fmt, 0).ok_or("short fmt")?;
    let channels = u16_at(fmt, 2).ok_or("short fmt")?.max(1);
    let rate = u32_at(fmt, 4).ok_or("short fmt")?;
    let bits = u16_at(fmt, 14).unwrap_or(16);
    let samples: Vec<i16> = match (tag, bits) {
        (1, 8) => data.iter().map(|&b| ((b as i16) - 128) << 8).collect(),
        (1, 16) => data.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect(),
        (2, _) => adpcm(fmt, data, channels)?,
        _ => return Err(format!("unsupported format tag {tag}, {bits} bits")),
    };
    Ok(MapSoundClip {
        rate,
        channels,
        samples: samples.into(),
        loop_start,
    })
}

/// Microsoft ADPCM: blocks of a header (predictor, delta, two samples per
/// channel) then 4-bit nibbles, high nibble first, channels interleaved.
fn adpcm(fmt: &[u8], data: &[u8], channels: u16) -> Result<Vec<i16>, String> {
    const ADAPT: [i32; 16] = [
        230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230,
    ];
    let block = u16_at(fmt, 12).ok_or("short fmt")? as usize;
    let count = u16_at(fmt, 20).unwrap_or(7) as usize;
    let coefs: Vec<(i32, i32)> = (0..count)
        .map(|i| {
            (
                i16_at(fmt, 22 + i * 4).unwrap_or(0) as i32,
                i16_at(fmt, 24 + i * 4).unwrap_or(0) as i32,
            )
        })
        .collect();
    let ch = channels as usize;
    if block == 0 || coefs.is_empty() || ch > 2 {
        return Err("bad ADPCM header".into());
    }
    let mut out = Vec::new();
    for b in data.chunks(block) {
        if b.len() < 7 * ch {
            break;
        }
        let mut pred = vec![(0i32, 0i32); ch];
        let mut delta = vec![0i32; ch];
        let (mut s1, mut s2) = (vec![0i32; ch], vec![0i32; ch]);
        for c in 0..ch {
            let p = (b[c] as usize).min(coefs.len() - 1);
            pred[c] = coefs[p];
            delta[c] = i16_at(b, ch + c * 2).unwrap_or(0) as i32;
            s1[c] = i16_at(b, 3 * ch + c * 2).unwrap_or(0) as i32;
            s2[c] = i16_at(b, 5 * ch + c * 2).unwrap_or(0) as i32;
        }
        for c in 0..ch {
            out.push(s2[c] as i16);
        }
        for c in 0..ch {
            out.push(s1[c] as i16);
        }
        let mut c = 0;
        for &byte in &b[7 * ch..] {
            for nibble in [byte >> 4, byte & 0x0f] {
                let n = if nibble & 8 != 0 {
                    nibble as i32 - 16
                } else {
                    nibble as i32
                };
                let (c1, c2) = pred[c];
                let guess = (s1[c] * c1 + s2[c] * c2) / 256;
                let sample = (guess + n * delta[c]).clamp(-32768, 32767);
                out.push(sample as i16);
                s2[c] = s1[c];
                s1[c] = sample;
                delta[c] = (ADAPT[nibble as usize] * delta[c] / 256).max(16);
                c = (c + 1) % ch;
            }
        }
    }
    Ok(out)
}
