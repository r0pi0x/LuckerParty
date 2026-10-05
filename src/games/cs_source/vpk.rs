//! Valve VPK archives, versions 1 and 2, from the public format description
//! (developer.valvesoftware.com/wiki/VPK_File_Format).
//!
//! A `<name>_dir.vpk` holds a directory tree and sometimes file data; most
//! data lives in numbered siblings `<name>_000.vpk`, `<name>_001.vpk`, ...

use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use crate::mount::{Entry, FileSource, normalize};

const SIGNATURE: u32 = 0x55aa_1234;
/// Archive index meaning "data follows the tree in the _dir file".
const IN_DIR_FILE: u16 = 0x7fff;

#[derive(Clone, Debug)]
struct VpkEntry {
    crc: u32,
    preload: Vec<u8>,
    archive: u16,
    offset: u32,
    length: u32,
}

pub struct Vpk {
    dir_path: PathBuf,
    /// Path prefix of the numbered archives: `.../cstrike_pak` for
    /// `.../cstrike_pak_dir.vpk`.
    stem: PathBuf,
    /// Offset of in-dir file data: header plus tree.
    data_start: u64,
    entries: HashMap<String, VpkEntry>,
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn bad(&self, what: &str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, format!("VPK: {what} at byte {}", self.pos))
    }

    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.data.len())
            .ok_or_else(|| self.bad("truncated"))?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn cstr(&mut self) -> io::Result<String> {
        let len = self.data[self.pos..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| self.bad("unterminated string"))?;
        let s = String::from_utf8_lossy(&self.data[self.pos..self.pos + len]).into_owned();
        self.pos += len + 1;
        Ok(s)
    }
}

impl Vpk {
    pub fn open(dir_path: impl AsRef<Path>) -> io::Result<Self> {
        let dir_path = dir_path.as_ref().to_path_buf();
        let file_name = dir_path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let stem_name = file_name.strip_suffix("_dir.vpk").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("not a _dir.vpk: {}", dir_path.display()),
            )
        })?;
        let stem = dir_path.with_file_name(stem_name);

        let mut file = File::open(&dir_path)?;
        let mut header = [0u8; 12];
        file.read_exact(&mut header)?;
        let mut c = Cursor { data: &header, pos: 0 };
        if c.u32()? != SIGNATURE {
            return Err(c.bad("bad signature"));
        }
        let version = c.u32()?;
        let tree_size = c.u32()? as usize;
        let header_size = match version {
            1 => 12,
            2 => 28,
            v => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("VPK: unsupported version {v}"),
                ));
            }
        };
        file.seek(SeekFrom::Start(header_size))?;
        let mut tree = vec![0u8; tree_size];
        file.read_exact(&mut tree)?;

        let mut c = Cursor { data: &tree, pos: 0 };
        let mut entries = HashMap::new();
        // Tree: extension { directory { file entry }* }* ; each level ends
        // with an empty string. A single space means "none".
        loop {
            let ext = c.cstr()?;
            if ext.is_empty() {
                break;
            }
            loop {
                let dir = c.cstr()?;
                if dir.is_empty() {
                    break;
                }
                loop {
                    let name = c.cstr()?;
                    if name.is_empty() {
                        break;
                    }
                    let crc = c.u32()?;
                    let preload_len = c.u16()? as usize;
                    let archive = c.u16()?;
                    let offset = c.u32()?;
                    let length = c.u32()?;
                    if c.u16()? != 0xffff {
                        return Err(c.bad("bad entry terminator"));
                    }
                    let preload = c.take(preload_len)?.to_vec();
                    let mut path = String::new();
                    if dir != " " {
                        path.push_str(&dir);
                        path.push('/');
                    }
                    path.push_str(&name);
                    if ext != " " {
                        path.push('.');
                        path.push_str(&ext);
                    }
                    entries.insert(
                        normalize(&path),
                        VpkEntry {
                            crc,
                            preload,
                            archive,
                            offset,
                            length,
                        },
                    );
                }
            }
        }

        Ok(Self {
            dir_path,
            stem,
            data_start: header_size + tree_size as u64,
            entries,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// CRC32 the archive records for `path`, for integrity checks.
    pub fn crc(&self, path: &str) -> Option<u32> {
        self.entries.get(&normalize(path)).map(|e| e.crc)
    }

    fn read_entry(&self, e: &VpkEntry) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(e.preload.len() + e.length as usize);
        out.extend_from_slice(&e.preload);
        if e.length > 0 {
            let (path, offset) = if e.archive == IN_DIR_FILE {
                (self.dir_path.clone(), self.data_start + e.offset as u64)
            } else {
                let mut p = self.stem.clone().into_os_string();
                p.push(format!("_{:03}.vpk", e.archive));
                (PathBuf::from(p), e.offset as u64)
            };
            let mut f = File::open(&path)?;
            f.seek(SeekFrom::Start(offset))?;
            let start = out.len();
            out.resize(start + e.length as usize, 0);
            f.read_exact(&mut out[start..])?;
        }
        Ok(out)
    }
}

impl FileSource for Vpk {
    fn name(&self) -> String {
        self.dir_path.display().to_string()
    }

    fn entries(&self) -> Vec<Entry> {
        self.entries
            .iter()
            .map(|(path, e)| Entry {
                path: path.clone(),
                size: e.preload.len() as u64 + e.length as u64,
            })
            .collect()
    }

    fn read(&self, path: &str) -> Option<io::Result<Vec<u8>>> {
        self.entries.get(path).map(|e| self.read_entry(e))
    }
}
