//! LZMA-compressed BSP lumps (public BSP description, "Lump compression";
//! written by `bspzip -repack`, common on community maps). A compressed
//! lump's directory entry keeps its uncompressed size in the fourCC field
//! and its data starts with Source's LZMA header: "LZMA", the uncompressed
//! size (u32), the compressed size (u32), the 5 LZMA property bytes, then
//! the stream. vbsp decompresses the lumps it parses; our own lump readers
//! (lighting, ambient cubes, world lights, leaves, areas, occluders, ...)
//! read the file directly, so `inflate` hands them a plain file.

/// Lumps in a BSP v19-21 directory.
const LUMPS: usize = 64;
const ENTRY: usize = 16;
const DIRECTORY: usize = 8;

/// The file with every LZMA lump decompressed: each one is appended to
/// the end and its directory entry pointed at it (fourCC 0). Everything
/// else stays where it was: the game lump's sub-lumps carry absolute file
/// offsets. A lump that fails to decompress is left as it was (vbsp then
/// reports it if it parses that lump). Files without compressed lumps
/// come back unchanged.
pub fn inflate(mut bytes: Vec<u8>) -> Vec<u8> {
    // The compressed lumps, decompressed side by side (surf_sedona: 80 MB
    // of lumps, half of it its HDR lighting).
    let packed: Vec<(usize, &[u8])> = (0..LUMPS)
        .filter_map(|i| {
            let at = DIRECTORY + i * ENTRY;
            let entry = bytes.get(at..at + ENTRY)?;
            let word = |k: usize| u32::from_le_bytes(entry[k * 4..k * 4 + 4].try_into().unwrap()) as usize;
            let (ofs, len, four_cc) = (word(0), word(1), word(3));
            if four_cc == 0 || len < 17 {
                return None;
            }
            Some((i, bytes.get(ofs..ofs + len).filter(|r| r.starts_with(b"LZMA"))?))
        })
        .collect();
    let plain: Vec<(usize, Vec<u8>)> = std::thread::scope(|s| {
        let jobs: Vec<_> = packed
            .iter()
            .map(|&(i, raw)| s.spawn(move || decompress(raw).map(|p| (i, p))))
            .collect();
        jobs.into_iter().filter_map(|j| j.join().ok().flatten()).collect()
    });
    for (i, plain) in plain {
        let at = DIRECTORY + i * ENTRY;
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        let new_ofs = bytes.len() as u32;
        let new_len = plain.len() as u32;
        bytes.extend_from_slice(&plain);
        bytes[at..at + 4].copy_from_slice(&new_ofs.to_le_bytes());
        bytes[at + 4..at + 8].copy_from_slice(&new_len.to_le_bytes());
        bytes[at + 12..at + 16].copy_from_slice(&0u32.to_le_bytes());
    }
    bytes
}

/// One lump's data from its Source LZMA header.
fn decompress(raw: &[u8]) -> Option<Vec<u8>> {
    let actual = u32::from_le_bytes(raw[4..8].try_into().ok()?);
    let packed = u32::from_le_bytes(raw[8..12].try_into().ok()?) as usize;
    let stream = raw.get(12..17 + packed)?;
    let mut out = Vec::with_capacity(actual as usize);
    lzma_rs::lzma_decompress_with_options(
        &mut std::io::Cursor::new(stream),
        &mut out,
        &lzma_rs::decompress::Options {
            unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(Some(actual as u64)),
            allow_incomplete: false,
            memlimit: None,
        },
    )
    .ok()?;
    (out.len() == actual as usize).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lump in Source's LZMA form.
    fn source_lzma(plain: &[u8]) -> Vec<u8> {
        let mut alone = Vec::new();
        lzma_rs::lzma_compress(&mut std::io::Cursor::new(plain), &mut alone).unwrap();
        // .lzma: 5 property bytes, 8 size bytes, the stream.
        let stream = &alone[13..];
        let mut out = b"LZMA".to_vec();
        out.extend_from_slice(&(plain.len() as u32).to_le_bytes());
        out.extend_from_slice(&(stream.len() as u32).to_le_bytes());
        out.extend_from_slice(&alone[..5]);
        out.extend_from_slice(stream);
        out
    }

    fn lump(bytes: &[u8], i: usize) -> &[u8] {
        let at = DIRECTORY + i * ENTRY;
        let w = |k: usize| u32::from_le_bytes(bytes[at + k * 4..at + k * 4 + 4].try_into().unwrap()) as usize;
        &bytes[w(0)..w(0) + w(1)]
    }

    #[test]
    fn compressed_lumps_read_as_plain_data() {
        let lighting: Vec<u8> = (0..4000u32).map(|i| (i % 7) as u8).collect();
        let packed = source_lzma(&lighting);
        let plain_lump = b"plain".to_vec();
        let header = DIRECTORY + LUMPS * ENTRY + 4;
        let mut file = vec![0u8; header];
        file[..4].copy_from_slice(b"VBSP");
        file[4..8].copy_from_slice(&20u32.to_le_bytes());
        let put = |file: &mut Vec<u8>, i: usize, data: &[u8], four_cc: u32| {
            let ofs = file.len() as u32;
            file.extend_from_slice(data);
            let at = DIRECTORY + i * ENTRY;
            file[at..at + 4].copy_from_slice(&ofs.to_le_bytes());
            file[at + 4..at + 8].copy_from_slice(&(data.len() as u32).to_le_bytes());
            file[at + 12..at + 16].copy_from_slice(&four_cc.to_le_bytes());
        };
        put(&mut file, 8, &packed, lighting.len() as u32);
        put(&mut file, 35, &plain_lump, 0);
        let before = file.clone();

        let inflated = inflate(file);
        assert_eq!(lump(&inflated, 8), &lighting[..]);
        assert_eq!(lump(&inflated, 35), b"plain", "plain lumps stay put");
        assert_eq!(&inflated[..before.len()][header..], &before[header..], "nothing moves");
        // A file without compressed lumps comes back as it was.
        assert_eq!(inflate(inflated.clone()), inflated);
    }
}
