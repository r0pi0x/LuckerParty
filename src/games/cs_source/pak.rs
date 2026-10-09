//! A map's pakfile (BSP lump 40, a zip archive; public zip format,
//! APPNOTE.TXT) read straight from the BSP's bytes. Repacked community
//! maps (`bspzip -repack`) store their files LZMA-compressed (zip method
//! 14); reading those through vbsp's zip reader decoded them at a few MB/s
//! (surf_sedona: 160 MB of packed files, a minute of its load). Here
//! stored and LZMA entries are read directly (LZMA by `lzma-rs`), several
//! at once when asked for together (`prefetch`); other methods are left
//! to the caller's fallback.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Zip compression methods.
const STORED: u16 = 0;
const LZMA: u16 = 14;

struct Entry {
    method: u16,
    /// Offset of the entry's local header in the archive.
    local: usize,
    packed: usize,
    size: usize,
}

pub struct Pak<'a> {
    bytes: &'a [u8],
    /// Entries by exact name and by lower-case name with `/` separators.
    entries: Vec<Entry>,
    by_name: HashMap<String, usize>,
    by_lower: HashMap<String, usize>,
    /// Files already decoded by `prefetch`, taken by the first `read`.
    fetched: Mutex<HashMap<usize, Arc<Vec<u8>>>>,
}

impl<'a> Pak<'a> {
    /// The archive's directory (empty when `bytes` isn't a zip).
    pub fn new(bytes: &'a [u8]) -> Self {
        let mut pak = Self {
            bytes,
            entries: Vec::new(),
            by_name: HashMap::new(),
            by_lower: HashMap::new(),
            fetched: Mutex::new(HashMap::new()),
        };
        pak.read_directory();
        pak
    }

    fn read_directory(&mut self) -> Option<()> {
        let b = self.bytes;
        let u16_at = |at: usize| Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?));
        let u32_at = |at: usize| Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize);
        // End of central directory: signature, then 18 bytes and a comment
        // of up to 64 KiB.
        let lowest = b.len().saturating_sub(22 + 0xFFFF);
        let end = (lowest..=b.len().checked_sub(22)?)
            .rev()
            .find(|&at| b[at..at + 4] == [0x50, 0x4b, 0x05, 0x06])?;
        let count = u16_at(end + 10)?;
        let mut at = u32_at(end + 16)?;
        for _ in 0..count {
            if b.get(at..at + 4)? != [0x50, 0x4b, 0x01, 0x02] {
                return None;
            }
            let method = u16_at(at + 10)?;
            let packed = u32_at(at + 20)?;
            let size = u32_at(at + 24)?;
            let (name_len, extra_len, comment_len) = (
                u16_at(at + 28)? as usize,
                u16_at(at + 30)? as usize,
                u16_at(at + 32)? as usize,
            );
            let local = u32_at(at + 42)?;
            let name = String::from_utf8_lossy(b.get(at + 46..at + 46 + name_len)?).into_owned();
            let index = self.entries.len();
            self.entries.push(Entry {
                method,
                local,
                packed,
                size,
            });
            self.by_lower.insert(name.replace('\\', "/").to_lowercase(), index);
            self.by_name.insert(name, index);
            at += 46 + name_len + extra_len + comment_len;
        }
        Some(())
    }

    /// Every entry's name, lower case with `/` separators.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_lower.keys().map(String::as_str)
    }

    /// The entry for `name`: exactly, else by lower-case name.
    fn find(&self, name: &str) -> Option<usize> {
        self.by_name
            .get(name)
            .or_else(|| self.by_lower.get(&name.replace('\\', "/").to_lowercase()))
            .copied()
    }

    /// Whether the archive has `name` (any case).
    pub fn contains(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    /// A file's contents (any case). `Err` for entries this reader doesn't
    /// decode (other compression methods, damaged data): the caller can
    /// try another zip reader.
    pub fn read(&self, name: &str) -> Option<Result<Vec<u8>, ()>> {
        let index = self.find(name)?;
        if let Some(data) = self.fetched.lock().unwrap_or_else(|e| e.into_inner()).remove(&index) {
            return Some(Ok(Arc::unwrap_or_clone(data)));
        }
        Some(self.decode(index).ok_or(()))
    }

    fn decode(&self, index: usize) -> Option<Vec<u8>> {
        let e = &self.entries[index];
        let b = self.bytes;
        let header = b.get(e.local..e.local + 30)?;
        if header[..4] != [0x50, 0x4b, 0x03, 0x04] {
            return None;
        }
        let name_len = u16::from_le_bytes([header[26], header[27]]) as usize;
        let extra_len = u16::from_le_bytes([header[28], header[29]]) as usize;
        let start = e.local + 30 + name_len + extra_len;
        let raw = b.get(start..start + e.packed)?;
        match e.method {
            STORED => (raw.len() == e.size).then(|| raw.to_vec()),
            // Zip's LZMA: a version (2 bytes), the property size (2, always
            // 5), the properties (5), then the stream.
            LZMA => {
                if raw.get(2..4)? != [5, 0] {
                    return None;
                }
                let mut out = Vec::with_capacity(e.size);
                lzma_rs::lzma_decompress_with_options(
                    &mut std::io::Cursor::new(&raw[4..]),
                    &mut out,
                    &lzma_rs::decompress::Options {
                        unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(Some(e.size as u64)),
                        allow_incomplete: false,
                        memlimit: None,
                    },
                )
                .ok()?;
                (out.len() == e.size).then_some(out)
            }
            _ => None,
        }
    }

    /// Decode these files (those present, not yet fetched) on several
    /// threads, for the `read`s that follow.
    pub fn prefetch<'n>(&self, names: impl IntoIterator<Item = &'n str>) {
        let mut wanted: Vec<usize> = names.into_iter().filter_map(|n| self.find(n)).collect();
        wanted.sort_unstable();
        wanted.dedup();
        {
            let fetched = self.fetched.lock().unwrap_or_else(|e| e.into_inner());
            wanted.retain(|i| !fetched.contains_key(i) && self.entries[*i].method == LZMA);
        }
        if wanted.is_empty() {
            return;
        }
        // Largest first, so the threads finish together.
        wanted.sort_by_key(|i| std::cmp::Reverse(self.entries[*i].packed));
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..threads {
                s.spawn(|| {
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&index) = wanted.get(k) else { break };
                        if let Some(data) = self.decode(index) {
                            self.fetched
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(index, Arc::new(data));
                        }
                    }
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zip with the given (name, method, stored data, uncompressed size)
    /// entries.
    fn zip(entries: &[(&str, u16, Vec<u8>, usize)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, method, data, size) in entries {
            let local = out.len() as u32;
            out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 8]);
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(*size as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);
            central.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(*size as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&local.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let at = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
        let n = (entries.len() as u16).to_le_bytes();
        out.extend_from_slice(&[n[0], n[1], n[0], n[1]]);
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&at.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    /// `plain` as a zip LZMA entry's data.
    fn zip_lzma(plain: &[u8]) -> Vec<u8> {
        let mut alone = Vec::new();
        lzma_rs::lzma_compress(&mut std::io::Cursor::new(plain), &mut alone).unwrap();
        // .lzma: 5 property bytes, 8 size bytes, the stream.
        let mut out = vec![9, 20, 5, 0];
        out.extend_from_slice(&alone[..5]);
        out.extend_from_slice(&alone[13..]);
        out
    }

    #[test]
    fn reads_stored_and_lzma_entries_in_any_case() {
        let text: Vec<u8> = (0..5000u32).map(|i| (i % 13) as u8).collect();
        let bytes = zip(&[
            ("materials/Maps/a.vmt", STORED, b"plain".to_vec(), 5),
            ("materials/b.vtf", LZMA, zip_lzma(&text), text.len()),
            ("c.txt", 8, vec![1, 2, 3], 9),
        ]);
        let pak = Pak::new(&bytes);
        assert_eq!(pak.read("materials/maps/A.vmt"), Some(Ok(b"plain".to_vec())));
        assert_eq!(pak.read("materials/b.vtf"), Some(Ok(text.clone())));
        assert_eq!(pak.read("c.txt"), Some(Err(())), "deflate: left to the fallback");
        assert_eq!(pak.read("missing"), None);
        pak.prefetch(["materials/b.vtf", "missing"]);
        assert_eq!(pak.read("materials/b.vtf"), Some(Ok(text)), "prefetched");
        assert!(Pak::new(b"not a zip").read("x").is_none());
    }
}
