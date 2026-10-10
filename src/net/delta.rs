//! Byte deltas: a message encoded against an earlier one both sides have
//! (`OwnState` against the last one the client acknowledged, `server`'s
//! `send_own_states`), as Source's snapshots are deltas against the last
//! acknowledged one (Valve Developer Wiki, "Source Multiplayer
//! Networking"). The receiver rebuilds the exact bytes: what it decodes
//! is bit for bit what the sender encoded.
//!
//! Format: the new length (varint), then runs to the end of it: bytes
//! equal to the base's at the same offset to skip (varint), bytes to take
//! from the message (varint, then the bytes). Equal stretches shorter
//! than `MIN_SKIP` are carried as bytes (a run costs two varints).

/// Equal bytes worth a run of their own.
const MIN_SKIP: usize = 3;

fn put_varint(out: &mut Vec<u8>, mut v: usize) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn get_varint(data: &mut &[u8]) -> Option<usize> {
    let mut v: usize = 0;
    for shift in (0..64).step_by(7) {
        let (&b, rest) = data.split_first()?;
        *data = rest;
        v |= ((b & 0x7f) as usize).checked_shl(shift)?;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

/// `new` as a delta against `base`.
pub fn encode(base: &[u8], new: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    put_varint(&mut out, new.len());
    let same = |i: usize| base.get(i) == Some(&new[i]);
    let mut i = 0;
    while i < new.len() {
        // Equal bytes to skip.
        let start = i;
        while i < new.len() && same(i) {
            i += 1;
        }
        let skip = i - start;
        if i == new.len() {
            // Equal to the end: nothing more to say.
            if skip > 0 || start == 0 {
                put_varint(&mut out, skip);
                put_varint(&mut out, 0);
            }
            break;
        }
        // Bytes to send, through equal stretches too short for a run.
        let lit = i;
        loop {
            while i < new.len() && !same(i) {
                i += 1;
            }
            let mut j = i;
            while j < new.len() && same(j) && j - i < MIN_SKIP {
                j += 1;
            }
            if j < new.len() && j - i < MIN_SKIP {
                i = j;
                continue;
            }
            break;
        }
        put_varint(&mut out, skip);
        put_varint(&mut out, i - lit);
        out.extend_from_slice(&new[lit..i]);
    }
    out
}

/// The bytes `encode(base, new)` was made from; None for a delta that
/// doesn't fit `base` (another base, or damaged).
pub fn decode(base: &[u8], mut delta: &[u8]) -> Option<Vec<u8>> {
    let len = get_varint(&mut delta)?;
    if len > (1 << 24) {
        return None;
    }
    let mut out = Vec::with_capacity(len);
    while out.len() < len || !delta.is_empty() {
        let skip = get_varint(&mut delta)?;
        let at = out.len();
        out.extend_from_slice(base.get(at..at.checked_add(skip)?)?);
        let n = get_varint(&mut delta)?;
        let (bytes, rest) = delta.split_at_checked(n)?;
        out.extend_from_slice(bytes);
        delta = rest;
        if out.len() > len {
            return None;
        }
        if skip == 0 && n == 0 && out.len() < len {
            return None;
        }
    }
    (out.len() == len).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(base: &[u8], new: &[u8]) -> usize {
        let d = encode(base, new);
        assert_eq!(
            decode(base, &d).as_deref(),
            Some(new),
            "base {base:?} new {new:?} delta {d:?}"
        );
        d.len()
    }

    #[test]
    fn rebuilds_the_exact_bytes() {
        let base: Vec<u8> = (0..200u8).collect();
        // Unchanged: the length and one run.
        assert_eq!(round_trip(&base, &base), 5);
        // A few scattered changes.
        let mut new = base.clone();
        new[10] = 0;
        new[11] = 0;
        new[100] ^= 0xff;
        new[199] = 7;
        assert!(round_trip(&base, &new) < 16);
        // Longer, shorter, empty, from nothing.
        let mut longer = base.clone();
        longer.extend_from_slice(&[1, 2, 3]);
        round_trip(&base, &longer);
        round_trip(&base, &base[..50]);
        round_trip(&base, &[]);
        round_trip(&[], &base);
        round_trip(&[], &[]);
        // Equal stretches shorter than a run ride along.
        let mut close = base.clone();
        for i in (20..60).step_by(2) {
            close[i] ^= 1;
        }
        assert!(round_trip(&base, &close) < 50);
        // Pseudo-random edits.
        let mut x = 0x1234_5678u32;
        for _ in 0..500 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let mut new = base.clone();
            new.truncate(100 + (x % 150) as usize);
            new.resize(100 + (x % 150) as usize, 9);
            for k in 0..(x % 20) {
                let i = ((x >> 3).wrapping_mul(k + 1) as usize) % new.len();
                new[i] = new[i].wrapping_add(k as u8 + 1);
            }
            round_trip(&base, &new);
        }
    }

    #[test]
    fn a_delta_for_another_base_fails() {
        let base: Vec<u8> = (0..100u8).collect();
        let mut new = base.clone();
        new[50] = 0;
        let d = encode(&base, &new);
        assert_eq!(decode(&base[..20], &d), None);
        assert_eq!(decode(&base, &d[..d.len() - 1]), None);
        assert_eq!(decode(&base, &[]), None);
    }
}
