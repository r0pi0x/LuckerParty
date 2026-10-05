//! CS:S navigation meshes (`maps/<map>.nav`, versions 9 and 16) into the
//! neutral `NavMesh`, per specs/cs_source/nav.md "File layout".

use bevy::prelude::*;

use super::movement::to_engine;
use crate::map::nav::{NavArea, NavLadder, NavMesh, Side, Via};

const MAGIC: u32 = 0xFEED_FACE;
/// Runtime-only flag bits that should never be in a file.
const RUNTIME_FLAGS: u32 = 0xE000_0000;

/// What the parser saw beyond the mesh itself, for tests and diagnostics.
#[derive(Clone, Debug, Default)]
pub struct NavInfo {
    pub version: u32,
    pub subversion: u32,
    pub hiding_spots: usize,
    pub encounters: usize,
    /// Bytes left after the last record (0 for a well-formed file).
    pub trailing: usize,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let s = self
            .b
            .get(self.at..self.at + n)
            .ok_or_else(|| format!("nav: truncated at byte {}", self.at))?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn vec3(&mut self) -> Result<Vec3, String> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
}

/// Source compass side (N=0 is -Y) to the engine edge it becomes.
fn side(d: usize) -> Side {
    match d {
        0 => Side::MaxZ,
        1 => Side::MaxX,
        2 => Side::MinZ,
        _ => Side::MinX,
    }
}

struct RawArea {
    id: u32,
    flags: u32,
    nw: Vec3,
    se: Vec3,
    ne_z: f32,
    sw_z: f32,
    links: [Vec<u32>; 4],
    place: u16,
    ladders: [Vec<u32>; 2],
}

pub fn parse(bytes: &[u8]) -> Result<(NavMesh, NavInfo), String> {
    let mut r = Reader { b: bytes, at: 0 };
    if r.u32()? != MAGIC {
        return Err("nav: bad magic".into());
    }
    let v = r.u32()?;
    if v > 16 {
        return Err(format!("nav: version {v} is newer than 16"));
    }
    let mut info = NavInfo {
        version: v,
        ..default()
    };
    if v >= 10 {
        info.subversion = r.u32()?;
    }
    if v >= 4 {
        r.u32()?; // size of the BSP it was built for (advisory)
    }
    if v >= 14 {
        r.u8()?; // analysed
    }
    let mut places = Vec::new();
    if v >= 5 {
        for _ in 0..r.u16()? {
            let len = r.u16()? as usize;
            let raw = r.take(len)?;
            let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
            places.push(String::from_utf8_lossy(&raw[..end]).into_owned());
        }
    }
    if v >= 12 {
        r.u8()?; // has unnamed areas
    }

    let count = r.u32()? as usize;
    let mut raw = Vec::with_capacity(count);
    for _ in 0..count {
        let id = r.u32()?;
        let flags = match v {
            ..=8 => r.u8()? as u32,
            9..=12 => r.u16()? as u32,
            _ => r.u32()?,
        } & !RUNTIME_FLAGS;
        let (nw, se) = (r.vec3()?, r.vec3()?);
        let (ne_z, sw_z) = (r.f32()?, r.f32()?);
        let mut links: [Vec<u32>; 4] = default();
        for side in &mut links {
            for _ in 0..r.u32()? {
                side.push(r.u32()?);
            }
        }
        let spots = r.u8()? as usize;
        info.hiding_spots += spots;
        r.take(spots * if v >= 2 { 17 } else { 12 })?;
        if v < 15 {
            let approaches = r.u8()? as usize;
            r.take(approaches * 14)?;
        }
        let encounters = r.u32()? as usize;
        info.encounters += encounters;
        if v >= 3 {
            for _ in 0..encounters {
                r.take(10)?;
                let n = r.u8()? as usize;
                r.take(n * 5)?;
            }
        } else if encounters > 0 {
            return Err("nav: version < 3 encounters unsupported".into());
        }
        let place = if v >= 5 { r.u16()? } else { 0 };
        let mut ladders: [Vec<u32>; 2] = default();
        if v >= 7 {
            for dir in &mut ladders {
                for _ in 0..r.u32()? {
                    let l = r.u32()?;
                    if !dir.contains(&l) {
                        dir.push(l);
                    }
                }
            }
        }
        if v >= 8 {
            r.take(8)?; // earliest occupy times
        }
        if v >= 11 {
            r.take(16)?; // corner light
        }
        if v >= 16 {
            let visible = r.u32()? as usize;
            r.take(visible * 5 + 4)?;
        }
        if v >= 10 && info.subversion >= 1 {
            // CS:S's own per-area data: a count, 0 in every shipped file
            // (spec open question 1: entry size assumed 14).
            let n = r.u8()? as usize;
            r.take(n * 14)?;
        }
        raw.push(RawArea {
            id,
            flags,
            nw,
            se,
            ne_z,
            sw_z,
            links,
            place,
            ladders,
        });
    }

    let mut ladders = Vec::new();
    let mut ladder_ends: Vec<(u32, [u32; 3], u32)> = Vec::new();
    if v >= 6 {
        for _ in 0..r.u32()? {
            let id = r.u32()?;
            let _width = r.f32()?;
            let (top, bottom) = (r.vec3()?, r.vec3()?);
            let length = r.f32()?;
            let dir = r.u32()? as usize;
            if v == 6 {
                r.u8()?;
            }
            let (forward, left, right, _behind, bottom_area) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
            let normal = match dir {
                0 => Vec3::NEG_Y,
                1 => Vec3::X,
                2 => Vec3::Y,
                _ => Vec3::NEG_X,
            };
            ladders.push(NavLadder {
                top: to_engine(top),
                bottom: to_engine(bottom),
                length: length * 0.0254,
                normal: to_engine(normal).normalize(),
            });
            ladder_ends.push((id, [forward, left, right], bottom_area));
        }
    }
    info.trailing = bytes.len() - r.at;

    let index: std::collections::HashMap<u32, usize> = raw.iter().enumerate().map(|(i, a)| (a.id, i)).collect();
    let ladder_index = |id: u32| ladder_ends.iter().position(|l| l.0 == id);
    let areas = raw
        .iter()
        .map(|a| {
            let mut links = Vec::new();
            for (d, list) in a.links.iter().enumerate() {
                for target in list {
                    if *target != a.id
                        && let Some(&t) = index.get(target)
                    {
                        links.push((t, Via::Walk(side(d))));
                    }
                }
            }
            for l in a.ladders[0].iter().filter_map(|&id| ladder_index(id)) {
                for top in ladder_ends[l].1 {
                    if let Some(&t) = index.get(&top) {
                        links.push((t, Via::LadderUp(l)));
                    }
                }
            }
            for l in a.ladders[1].iter().filter_map(|&id| ladder_index(id)) {
                if let Some(&t) = index.get(&ladder_ends[l].2) {
                    links.push((t, Via::LadderDown(l)));
                }
            }
            let centre = (a.nw + a.se) / 2.0;
            NavArea {
                id: a.id,
                flags: a.flags,
                // Source y grows south; engine z = -y.
                min: Vec2::new(a.nw.x, -a.se.y) * 0.0254,
                max: Vec2::new(a.se.x, -a.nw.y) * 0.0254,
                heights: [a.sw_z, a.se.z, a.ne_z, a.nw.z].map(|h| h * 0.0254),
                center: to_engine(centre),
                links,
                place: (a.place > 0).then(|| a.place as usize - 1),
            }
        })
        .collect();
    Ok((NavMesh { areas, ladders, places }, info))
}
