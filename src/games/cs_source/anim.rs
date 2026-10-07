//! Model animations from `.mdl` files (versions 44–48:
//! specs/cs_source/mdl_v48.md): bones, animations, sequences, pose
//! parameters and include models, merged into one `AnimSet` for a target
//! model (specs/cs_source/animation.md §1–§3, §8). Animation data may sit
//! in sections and in external `.ani` blocks (HL2 content such as the
//! hostages' `humans/male_*` models, mdl_v48.md §8); data that is missing
//! falls back to the zero-frame cache (§6). Read from raw bytes.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::map::anim::{AnimSet, Animation, AutoLayer, Axis, BonePose, PoseParam, Sequence, Track, euler};

/// Reads a file from the game (path relative to the game root).
pub type Read<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn i32(&self, at: usize) -> Result<i32, String> {
        self.0
            .get(at..at + 4)
            .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
            .ok_or_else(|| format!("read past the end at {at}"))
    }
    fn f32(&self, at: usize) -> Result<f32, String> {
        Ok(f32::from_bits(self.i32(at)? as u32))
    }
    fn i16(&self, at: usize) -> Result<i16, String> {
        self.0
            .get(at..at + 2)
            .map(|b| i16::from_le_bytes(b.try_into().unwrap()))
            .ok_or_else(|| format!("read past the end at {at}"))
    }
    fn u16(&self, at: usize) -> Result<u16, String> {
        Ok(self.i16(at)? as u16)
    }
    fn u8(&self, at: usize) -> Result<u8, String> {
        self.0
            .get(at)
            .copied()
            .ok_or_else(|| format!("read past the end at {at}"))
    }
    fn vec3(&self, at: usize) -> Result<Vec3, String> {
        Ok(Vec3::new(self.f32(at)?, self.f32(at + 4)?, self.f32(at + 8)?))
    }
    fn string(&self, at: usize) -> String {
        let rest = self.0.get(at..).unwrap_or_default();
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        String::from_utf8_lossy(&rest[..end]).into_owned()
    }
    /// The string at `base + offset(at)`.
    fn name(&self, base: usize, at: usize) -> Result<String, String> {
        Ok(self.string((base as i64 + self.i32(at)? as i64) as usize))
    }
}

#[derive(Debug)]
struct Bone {
    name: String,
    parent: Option<usize>,
    position: Vec3,
    rotation: Quat,
    euler: Vec3,
    pos_scale: Vec3,
    rot_scale: Vec3,
    flags: u32,
}

/// Bone flags: the bone has zero-frame positions / rotations (§6).
const SAVEFRAME_POS: u32 = 0x0020_0000;
const SAVEFRAME_ROT: u32 = 0x0040_0000;

struct RawSequence {
    name: String,
    activity: String,
    activity_weight: i32,
    flags: i32,
    fade: (f32, f32),
    grid: (usize, usize),
    anims: Vec<usize>,
    params: [Option<usize>; 2],
    range: [(f32, f32); 2],
    keys: [Vec<f32>; 2],
    /// (sequence, start, peak, tail, end), indices local.
    autolayers: Vec<(usize, [f32; 4])>,
    weights: Vec<f32>,
    events: Vec<crate::map::anim::AnimEvent>,
}

/// One `.mdl` file's animation data, indices local to it.
struct Model {
    version: i32,
    bones: Vec<Bone>,
    /// Animation descriptions: name and byte offset.
    anims: Vec<(String, usize)>,
    sequences: Vec<RawSequence>,
    params: Vec<PoseParam>,
    includes: Vec<String>,
    bytes: Vec<u8>,
    /// The external animation file's name (empty: none), its bytes once
    /// read, and its block table (start, end; entry 0 stands for the
    /// `.mdl` itself).
    ani_name: String,
    ani: Option<Vec<u8>>,
    blocks: Vec<(usize, usize)>,
}

fn parse(bytes: Vec<u8>) -> Result<Model, String> {
    let b = Bytes(&bytes);
    let version = b.i32(4)?;
    // Versions 45-48 read like 44 for everything used here (spec
    // mdl_v48.md §1, §9: the extra header block is found by its offset;
    // the per-version differences are in `decode`).
    if !(44..=48).contains(&version) {
        return Err(format!("model version {version}, want 44 to 48"));
    }
    let count =
        |at| -> Result<(usize, usize), String> { Ok((b.i32(at)?.max(0) as usize, b.i32(at + 4)?.max(0) as usize)) };
    let (n, at) = count(156)?;
    let mut bones = Vec::with_capacity(n);
    for i in 0..n {
        let o = at + i * 216;
        let parent = b.i32(o + 4)?;
        bones.push(Bone {
            name: b.name(o, o)?,
            parent: (parent >= 0).then_some(parent as usize),
            position: b.vec3(o + 32)?,
            rotation: Quat::from_xyzw(b.f32(o + 44)?, b.f32(o + 48)?, b.f32(o + 52)?, b.f32(o + 56)?),
            euler: b.vec3(o + 60)?,
            pos_scale: b.vec3(o + 72)?,
            rot_scale: b.vec3(o + 84)?,
            flags: b.i32(o + 160)? as u32,
        });
    }
    let (n, at) = count(180)?;
    let anims = (0..n)
        .map(|i| {
            let o = at + i * 100;
            Ok((b.name(o, o + 4)?, o))
        })
        .collect::<Result<_, String>>()?;
    let (n, at) = count(300)?;
    let params = (0..n)
        .map(|i| {
            let o = at + i * 20;
            Ok(PoseParam {
                name: b.name(o, o)?,
                start: b.f32(o + 8)?,
                end: b.f32(o + 12)?,
                looping: b.f32(o + 16)?,
            })
        })
        .collect::<Result<_, String>>()?;
    let (n, at) = count(336)?;
    let includes = (0..n)
        .map(|i| {
            let o = at + i * 8;
            b.name(o, o + 4)
        })
        .collect::<Result<_, String>>()?;
    let (n, at) = count(188)?;
    let mut sequences = Vec::with_capacity(n);
    for i in 0..n {
        let o = at + i * 212;
        let (w, h) = (b.i32(o + 68)?.max(1) as usize, b.i32(o + 72)?.max(1) as usize);
        let grid_at = o + b.i32(o + 60)? as usize;
        let anims = (0..w * h)
            .map(|k| Ok(b.i16(grid_at + 2 * k)?.max(0) as usize))
            .collect::<Result<_, String>>()?;
        let param = |at| -> Result<Option<usize>, String> {
            let p = b.i32(at)?;
            Ok((p >= 0).then_some(p as usize))
        };
        let key_at = b.i32(o + 160)?;
        let keys = |axis: usize| -> Result<Vec<f32>, String> {
            if key_at == 0 {
                return Ok(Vec::new());
            }
            let (skip, len) = if axis == 0 { (0, w) } else { (w, h) };
            (0..len).map(|k| b.f32(o + key_at as usize + 4 * (skip + k))).collect()
        };
        let (layers, layer_at) = count(o + 148)?;
        let autolayers = (0..layers)
            .map(|k| {
                let l = o + layer_at + 24 * k;
                Ok((
                    b.i16(l)?.max(0) as usize,
                    [b.f32(l + 8)?, b.f32(l + 12)?, b.f32(l + 16)?, b.f32(l + 20)?],
                ))
            })
            .collect::<Result<_, String>>()?;
        // Events (80 bytes each): cycle, number, type, 64 bytes of
        // options, then the offset of a name for named events.
        let (n_events, events_at) = count(o + 24)?;
        let events = (0..n_events)
            .map(|k| {
                let e = o + events_at + 80 * k;
                let name_at = b.i32(e + 76)?;
                Ok(crate::map::anim::AnimEvent {
                    cycle: b.f32(e)?,
                    event: b.i32(e + 4)?,
                    name: if name_at != 0 {
                        b.name(e, e + 76)?
                    } else {
                        String::new()
                    },
                    options: {
                        let raw = b.0.get(e + 12..e + 76).unwrap_or_default();
                        let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
                        String::from_utf8_lossy(&raw[..end]).into_owned()
                    },
                })
            })
            .collect::<Result<_, String>>()?;
        let weight_at = o + b.i32(o + 156)? as usize;
        let weights = (0..bones.len())
            .map(|k| b.f32(weight_at + 4 * k))
            .collect::<Result<_, String>>()?;
        sequences.push(RawSequence {
            name: b.name(o, o + 4)?,
            activity: b.name(o, o + 8)?,
            activity_weight: b.i32(o + 20)?,
            flags: b.i32(o + 12)?,
            fade: (b.f32(o + 104)?, b.f32(o + 108)?),
            grid: (w, h),
            anims,
            params: [param(o + 76)?, param(o + 80)?],
            range: [(b.f32(o + 84)?, b.f32(o + 92)?), (b.f32(o + 88)?, b.f32(o + 96)?)],
            keys: [keys(0)?, keys(1)?],
            autolayers,
            weights,
            events,
        });
    }
    // The external animation file (§8): name and block table.
    let ani_at = b.i32(348)?;
    let ani_name = if ani_at > 0 {
        b.string(ani_at as usize)
    } else {
        String::new()
    };
    let (n, at) = count(352)?;
    let blocks = (0..n)
        .map(|i| {
            let o = at + i * 8;
            Ok((b.i32(o)?.max(0) as usize, b.i32(o + 4)?.max(0) as usize))
        })
        .collect::<Result<_, String>>()?;
    Ok(Model {
        version,
        bones,
        anims,
        sequences,
        params,
        includes,
        bytes,
        ani_name,
        ani: None,
        blocks,
    })
}

/// IEEE half float, with exponent 31 read as ±65504 (mantissa 0) or 0.
fn half(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = (h >> 10) & 0x1f;
    let mant = (h & 0x3ff) as f32;
    sign * match exp {
        0 => mant / 1024.0 * 2f32.powi(-14),
        31 if mant == 0.0 => 65504.0,
        31 => 0.0,
        e => (1.0 + mant / 1024.0) * 2f32.powi(e as i32 - 15),
    }
}

fn quat48(b: &Bytes, at: usize) -> Result<Quat, String> {
    let (a, bb, c) = (b.u16(at)?, b.u16(at + 2)?, b.u16(at + 4)?);
    let x = (a as f32 - 32768.0) / 32768.0;
    let y = (bb as f32 - 32768.0) / 32768.0;
    let z = ((c & 0x7fff) as f32 - 16384.0) / 16384.0;
    let w = (1.0 - x * x - y * y - z * z).max(0.0).sqrt();
    Ok(Quat::from_xyzw(x, y, z, if c & 0x8000 != 0 { -w } else { w }))
}

fn quat64(b: &Bytes, at: usize) -> Result<Quat, String> {
    let v = (b.i32(at)? as u32 as u64) | ((b.i32(at + 4)? as u32 as u64) << 32);
    let part = |shift: u32| ((v >> shift) & 0x1f_ffff) as f64;
    let x = ((part(0) - 1048576.0) / 1048576.5) as f32;
    let y = ((part(21) - 1048576.0) / 1048576.5) as f32;
    let z = ((part(42) - 1048576.0) / 1048576.5) as f32;
    let w = (1.0 - x * x - y * y - z * z).max(0.0).sqrt();
    Ok(Quat::from_xyzw(x, y, z, if v >> 63 != 0 { -w } else { w }))
}

fn vec48(b: &Bytes, at: usize) -> Result<Vec3, String> {
    Ok(Vec3::new(half(b.u16(at)?), half(b.u16(at + 2)?), half(b.u16(at + 4)?)))
}

/// Every frame's value of a run-length stream starting at `at` (§3).
fn stream(b: &Bytes, at: usize, frames: usize) -> Result<Vec<f32>, String> {
    let (v0, t0) = (b.u8(at)? as usize, b.u8(at + 1)? as usize);
    if v0 == 1 && t0 == 1 {
        return Ok(vec![b.i16(at + 2)? as f32; frames]);
    }
    let mut out = Vec::with_capacity(frames);
    let mut span = at;
    let mut k = 0;
    while out.len() < frames {
        let (v, t) = (b.u8(span)? as usize, b.u8(span + 1)? as usize);
        if t == 0 {
            out.resize(frames, 0.0);
            break;
        }
        if k >= t {
            span += 2 + 2 * v;
            k = 0;
            continue;
        }
        let i = k.min(v.max(1) - 1);
        out.push(if v == 0 { 0.0 } else { b.i16(span + 2 + 2 * i)? as f32 });
        k += 1;
    }
    Ok(out)
}

/// A run of an animation's frames stored together (animation.md §1,
/// mdl_v48.md §8): the block (0: the `.mdl`, > 0: the `.ani`, < 0:
/// missing), the records' offset, the section it came from, and which
/// frames.
struct Chunk {
    block: i32,
    offset: i64,
    section: usize,
    first: usize,
    count: usize,
}

/// An animation's zero-frame cache (mdl_v48.md §6): frames between
/// entries, and per bone (local index) its stored positions and rotations
/// (either may be empty).
#[derive(Debug, Clone)]
pub struct ZeroFrames {
    pub animation: String,
    pub span: usize,
    pub bones: Vec<(usize, Vec<Vec3>, Vec<Quat>)>,
}

/// The zero-frame cache of the animation description at `o` (version 48
/// only: §9 makes it junk or incompatible before).
fn zero_frames_at(m: &Model, o: usize) -> Result<Option<ZeroFrames>, String> {
    if m.version < 48 {
        return Ok(None);
    }
    let b = Bytes(&m.bytes);
    let span = b.i16(o + 88)?.max(0) as usize;
    let count = b.i16(o + 90)?.max(0) as usize;
    let at = b.i32(o + 92)?;
    if count == 0 || at <= 0 {
        return Ok(None);
    }
    let mut p = o + at as usize;
    let mut bones = Vec::new();
    for (i, bone) in m.bones.iter().enumerate() {
        let (mut ps, mut qs) = (Vec::new(), Vec::new());
        if bone.flags & SAVEFRAME_POS != 0 {
            for _ in 0..count {
                ps.push(vec48(&b, p)?);
                p += 6;
            }
        }
        if bone.flags & SAVEFRAME_ROT != 0 {
            for _ in 0..count {
                qs.push(quat64(&b, p)?);
                p += 8;
            }
        }
        if !ps.is_empty() || !qs.is_empty() {
            bones.push((i, ps, qs));
        }
    }
    Ok(Some(ZeroFrames {
        animation: String::new(),
        span,
        bones,
    }))
}

/// The value of a zero-frame curve at fractional frame `f` (§6: a
/// three-point Hermite curve; quaternions aligned to the next entry and
/// renormalized).
fn zero_sample(entries: &[Vec4], span: usize, f: f32, quat: bool) -> Vec4 {
    let c = entries.len();
    if c == 1 {
        return entries[0];
    }
    let span = span.max(1) as f32;
    let mut i = (f / span).floor().max(0.0) as usize;
    let s = if i >= c - 1 {
        i = c - 2;
        1.0
    } else {
        ((f - i as f32 * span) / span).clamp(0.0, 1.0)
    };
    let (mut p0, mut p1, p2) = (entries[i.saturating_sub(1)], entries[i], entries[(i + 1).min(c - 1)]);
    if quat {
        let align = |q: Vec4| {
            if (q - p2).length_squared() > (q + p2).length_squared() {
                -q
            } else {
                q
            }
        };
        p0 = align(p0);
        p1 = align(p1);
    }
    let (d1, d2) = (p1 - p0, p2 - p1);
    let (s2, s3) = (s * s, s * s * s);
    let h1 = 2.0 * s3 - 3.0 * s2 + 1.0;
    let (h2, h3, h4) = (1.0 - h1, s3 - 2.0 * s2 + s, s3 - s2);
    let r = p1 * h1 + p2 * h2 + d1 * h3 + d2 * h4;
    if quat { r.normalize_or_zero() } else { r }
}

/// The average ground speed of the animation description at `o`: the
/// horizontal distance its last movement segment ends at over the time to
/// its end frame (animation.md §11: 44-byte segments, end frame at +0,
/// position at +32).
fn movement_speed(b: &Bytes, o: usize) -> Result<f32, String> {
    let n = b.i32(o + 20)?.clamp(0, 64) as usize;
    let at = b.i32(o + 24)?;
    let fps = b.f32(o + 8)?;
    if n == 0 || at <= 0 || fps <= 0.0 {
        return Ok(0.0);
    }
    let last = o + at as usize + 44 * (n - 1);
    let end = b.i32(last)?;
    if end <= 0 {
        return Ok(0.0);
    }
    Ok(b.vec3(last + 32)?.truncate().length() / (end as f32 / fps))
}

/// Decode animation `a` of `m`, bone indices local to `m`.
fn decode(m: &Model, a: usize) -> Result<Animation, String> {
    let b = Bytes(&m.bytes);
    let (name, o) = (&m.anims[a].0, m.anims[a].1);
    let flags = b.i32(o + 12)?;
    let mut frames = b.i32(o + 16)?.max(1) as usize;
    let delta = flags & 0x4 != 0;
    let block = b.i32(o + 52)?;
    let offset = b.i32(o + 56)? as i64;
    let per_section = b.i32(o + 84)?.max(0) as usize;
    let mut chunks: Vec<Chunk> = Vec::new();
    if per_section == 0 {
        chunks.push(Chunk {
            block,
            offset,
            section: 0,
            first: 0,
            count: frames,
        });
    } else if m.version < 46 {
        // §9: sectioned data written by version 45 is unusable; one
        // frame, missing.
        frames = 1;
        chunks.push(Chunk {
            block: -1,
            offset: 0,
            section: 0,
            first: 0,
            count: 1,
        });
    } else {
        // §1: section ⌊f/S⌋ at local frame f − section·S; the last frame
        // of a longer animation is stored alone in section ⌊N/S⌋ + 1.
        let table = o as i64 + b.i32(o + 80)? as i64;
        for f in 0..frames {
            let section = if frames > per_section && f == frames - 1 {
                frames / per_section + 1
            } else {
                f / per_section
            };
            match chunks.last_mut() {
                Some(c) if c.section == section => c.count += 1,
                _ => {
                    let e = (table + 8 * section as i64) as usize;
                    chunks.push(Chunk {
                        block: b.i32(e)?,
                        offset: b.i32(e + 4)? as i64,
                        section,
                        first: f,
                        count: 1,
                    });
                }
            }
        }
    }
    let mut anim = Animation {
        name: name.clone(),
        fps: b.f32(o + 8)?,
        frames,
        delta,
        speed: movement_speed(&b, o)?,
        ..default()
    };
    // Each chunk's records (None: its data is missing).
    let mut parts: Vec<(&Chunk, Option<Vec<Track>>)> = Vec::with_capacity(chunks.len());
    for c in &chunks {
        let data: Option<(&[u8], i64)> = match c.block {
            0 => Some((&m.bytes, o as i64)),
            k if k > 0 => m
                .ani
                .as_deref()
                .zip(m.blocks.get(k as usize))
                .map(|(ani, (start, _))| (ani, *start as i64)),
            _ => None,
        };
        let tracks = match data {
            Some((bytes, base)) => {
                let at = base + c.offset;
                if at < 0 {
                    return Err(format!("{name}: records before the start of their block"));
                }
                Some(records(&Bytes(bytes), at as usize, c.count, &m.bones, name)?)
            }
            None => None,
        };
        parts.push((c, tracks));
    }
    if let [(_, Some(_))] = parts.as_slice() {
        anim.tracks = parts.pop().and_then(|p| p.1).unwrap_or_default();
        return Ok(anim);
    }
    // Several sections, or missing data: per bone and frame, then packed.
    let zero = if parts.iter().any(|p| p.1.is_none()) {
        zero_frames_at(m, o)?
    } else {
        None
    };
    let n = m.bones.len();
    let mut rot: Vec<Vec<Option<Quat>>> = vec![Vec::new(); n];
    let mut pos: Vec<Vec<Option<Vec3>>> = vec![Vec::new(); n];
    fn put<T: Copy>(v: &mut Vec<Option<T>>, frames: usize, f: usize, x: T) {
        if v.is_empty() {
            v.resize(frames, None);
        }
        v[f] = Some(x);
    }
    for (c, tracks) in &parts {
        match tracks {
            Some(tracks) => {
                for t in tracks {
                    for k in 0..c.count {
                        if let Some(r) = &t.rotation {
                            put(&mut rot[t.bone], frames, c.first + k, r[k.min(r.len() - 1)]);
                        }
                        if let Some(p) = &t.position {
                            put(&mut pos[t.bone], frames, c.first + k, p[k.min(p.len() - 1)]);
                        }
                    }
                }
            }
            None => {
                let Some(z) = &zero else { continue };
                for (bone, ps, qs) in &z.bones {
                    let ps: Vec<Vec4> = ps.iter().map(|p| p.extend(0.0)).collect();
                    let qs: Vec<Vec4> = qs.iter().map(|q| Vec4::from(*q)).collect();
                    for k in 0..c.count {
                        let f = (c.first + k) as f32;
                        if !ps.is_empty() {
                            put(
                                &mut pos[*bone],
                                frames,
                                c.first + k,
                                zero_sample(&ps, z.span, f, false).truncate(),
                            );
                        }
                        if !qs.is_empty() {
                            let q = Quat::from_vec4(zero_sample(&qs, z.span, f, true));
                            put(&mut rot[*bone], frames, c.first + k, q);
                        }
                    }
                }
            }
        }
    }
    fn pack<T: Copy + PartialEq>(v: Vec<Option<T>>, default: T) -> Option<Vec<T>> {
        if v.is_empty() {
            return None;
        }
        let v: Vec<T> = v.into_iter().map(|x| x.unwrap_or(default)).collect();
        if v.iter().all(|x| *x == v[0]) {
            Some(vec![v[0]])
        } else {
            Some(v)
        }
    }
    for (bone, (r, p)) in rot.into_iter().zip(pos).enumerate() {
        let info = &m.bones[bone];
        let (dr, dp) = if delta {
            (Quat::IDENTITY, Vec3::ZERO)
        } else {
            (info.rotation, info.position)
        };
        let track = Track {
            bone,
            rotation: pack(r, dr),
            position: pack(p, dp),
        };
        if track.rotation.is_some() || track.position.is_some() {
            anim.tracks.push(track);
        }
    }
    Ok(anim)
}

/// The per-bone records starting at `start` (animation.md §1, §3), for
/// `frames` frames from there.
fn records(b: &Bytes, start: usize, frames: usize, bones: &[Bone], name: &str) -> Result<Vec<Track>, String> {
    let mut tracks = Vec::new();
    let mut r = start;
    loop {
        let bone = b.u8(r)? as usize;
        if bone == 255 {
            break;
        }
        let rf = b.u8(r + 1)?;
        let next = b.i16(r + 2)?;
        let Some(info) = bones.get(bone) else {
            return Err(format!("{name}: bone {bone} out of range"));
        };
        let data = r + 4;
        let rdelta = rf & 0x10 != 0;
        let mut track = Track { bone, ..default() };
        if rf & 0x02 != 0 {
            track.rotation = Some(vec![quat48(&b, data)?]);
        } else if rf & 0x20 != 0 {
            track.rotation = Some(vec![quat64(&b, data)?]);
        } else if rf & 0x08 != 0 {
            let axes = (0..3)
                .map(|k| {
                    let off = b.i16(data + 2 * k)?;
                    if off <= 0 {
                        Ok(vec![0.0; frames])
                    } else {
                        stream(&b, data + off as usize, frames)
                    }
                })
                .collect::<Result<Vec<_>, String>>()?;
            let base = if rdelta { Vec3::ZERO } else { info.euler };
            track.rotation = Some(
                (0..frames)
                    .map(|f| {
                        let e = Vec3::new(axes[0][f], axes[1][f], axes[2][f]) * info.rot_scale + base;
                        euler(e.x, e.y, e.z)
                    })
                    .collect(),
            );
        } else if rdelta {
            track.rotation = Some(vec![Quat::IDENTITY]);
        }
        if rf & 0x01 != 0 {
            let at = data + if rf & 0x02 != 0 { 6 } else { 0 } + if rf & 0x20 != 0 { 8 } else { 0 };
            track.position = Some(vec![vec48(&b, at)?]);
        } else if rf & 0x04 != 0 {
            let triple = data + if rf & 0x08 != 0 { 6 } else { 0 };
            let axes = (0..3)
                .map(|k| {
                    let off = b.i16(triple + 2 * k)?;
                    if off <= 0 {
                        Ok(vec![0.0; frames])
                    } else {
                        stream(&b, triple + off as usize, frames)
                    }
                })
                .collect::<Result<Vec<_>, String>>()?;
            let base = if rdelta { Vec3::ZERO } else { info.position };
            track.position = Some(
                (0..frames)
                    .map(|f| Vec3::new(axes[0][f], axes[1][f], axes[2][f]) * info.pos_scale + base)
                    .collect(),
            );
        } else if rdelta {
            track.position = Some(vec![Vec3::ZERO]);
        }
        tracks.push(track);
        if next == 0 {
            break;
        }
        r = (r as i64 + next as i64) as usize;
    }
    Ok(tracks)
}

/// The target model's bones in its reference pose (name, parent, local
/// rotation and position).
/// A model's attachments (name, bone, transform in the bone's space) and
/// its illumination position (model space), from the header.
pub fn attachments(read: Read, path: &str) -> Result<(Vec<(String, usize, Transform)>, Vec3), String> {
    let bytes = read(path).ok_or_else(|| format!("{path}: not found"))?;
    let b = Bytes(&bytes);
    let illum = b.vec3(92)?;
    let (n, at) = (b.i32(240)?.max(0) as usize, b.i32(244)?.max(0) as usize);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        // Name offset, flags, bone, then a 3x4 matrix (rows; the last
        // column is the translation); 92 bytes each.
        let o = at + i * 92;
        let m = |r: usize, c: usize| b.f32(o + 12 + 4 * (r * 4 + c));
        let axes = Mat3::from_cols(
            Vec3::new(m(0, 0)?, m(1, 0)?, m(2, 0)?),
            Vec3::new(m(0, 1)?, m(1, 1)?, m(2, 1)?),
            Vec3::new(m(0, 2)?, m(1, 2)?, m(2, 2)?),
        );
        let translation = Vec3::new(m(0, 3)?, m(1, 3)?, m(2, 3)?);
        out.push((
            b.name(o, o)?,
            b.i32(o + 8)?.max(0) as usize,
            Transform::from_translation(translation).with_rotation(Quat::from_mat3(&axes)),
        ));
    }
    Ok((out, illum))
}

pub fn bones(read: Read, path: &str) -> Result<Vec<(String, Option<usize>, Quat, Vec3)>, String> {
    let m = parse(read(path).ok_or_else(|| format!("{path}: not found"))?)?;
    Ok(m.bones
        .into_iter()
        .map(|b| (b.name, b.parent, b.rotation, b.position))
        .collect())
}

/// The model at `path` with its external animation file read (§8; a
/// missing one leaves its animations to the zero-frame cache).
fn parse_with_blocks(read: Read, path: &str) -> Result<Model, String> {
    let mut m = parse(read(path).ok_or_else(|| format!("{path}: not found"))?).map_err(|e| format!("{path}: {e}"))?;
    if m.blocks.len() > 1 && !m.ani_name.is_empty() {
        let name = m.ani_name.replace('\\', "/");
        m.ani = read(&name).filter(|a| a.starts_with(b"IDAG"));
        if m.ani.is_none() {
            warn!("{path}: animation blocks {name} missing; using the zero-frame cache");
        }
    }
    Ok(m)
}

/// The zero-frame caches of the animations of the model at `path` (not
/// its includes), bone indices its own (mdl_v48.md §6).
pub fn zero_frames(read: Read, path: &str) -> Result<Vec<ZeroFrames>, String> {
    let m = parse(read(path).ok_or_else(|| format!("{path}: not found"))?)?;
    let mut out = Vec::new();
    for (name, o) in &m.anims {
        if let Some(mut z) = zero_frames_at(&m, *o)? {
            z.animation = name.clone();
            out.push(z);
        }
    }
    Ok(out)
}

/// Everything the model at `path` can play, through its include models
/// (§8). Bone indices are the model's own.
pub fn load(read: Read, path: &str) -> Result<AnimSet, String> {
    let mut models: Vec<Model> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    fn visit(read: Read, path: &str, models: &mut Vec<Model>, seen: &mut Vec<String>) -> Result<(), String> {
        let key = path.to_lowercase().replace('\\', "/");
        if seen.contains(&key) || seen.len() > 16 {
            return Ok(());
        }
        seen.push(key);
        let m = parse_with_blocks(read, path)?;
        let includes = m.includes.clone();
        models.push(m);
        for i in includes {
            visit(read, &i, models, seen)?;
        }
        Ok(())
    }
    visit(read, path, &mut models, &mut seen)?;
    let target = &models[0];
    let index: HashMap<String, usize> = target
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| (b.name.to_lowercase(), i))
        .collect();
    // Each model's bone → the target's.
    let bone_maps: Vec<Vec<Option<usize>>> = models
        .iter()
        .map(|m| {
            m.bones
                .iter()
                .map(|b| index.get(&b.name.to_lowercase()).copied())
                .collect()
        })
        .collect();
    let defaults: Vec<BonePose> = target.bones.iter().map(|b| (b.rotation, b.position)).collect();
    let bases = models
        .iter()
        .zip(&bone_maps)
        .map(|(m, map)| {
            let mut base = defaults.clone();
            for (b, t) in m.bones.iter().zip(map) {
                if let Some(t) = t {
                    base[*t] = (b.rotation, b.position);
                }
            }
            base
        })
        .collect();

    // Animations: first name wins.
    let mut set = AnimSet {
        defaults,
        bases,
        ..default()
    };
    let mut anim_names: HashMap<String, usize> = HashMap::new();
    let mut anim_maps: Vec<Vec<usize>> = Vec::new();
    for (mi, m) in models.iter().enumerate() {
        let mut local = Vec::with_capacity(m.anims.len());
        for (a, (name, _)) in m.anims.iter().enumerate() {
            let key = name.to_lowercase();
            if let Some(&i) = anim_names.get(&key) {
                local.push(i);
                continue;
            }
            let mut anim = decode(m, a)?;
            anim.base = mi;
            anim.tracks.retain_mut(|t| match bone_maps[mi][t.bone] {
                Some(b) => {
                    t.bone = b;
                    true
                }
                None => false,
            });
            anim_names.insert(key, set.animations.len());
            local.push(set.animations.len());
            set.animations.push(anim);
        }
        anim_maps.push(local);
    }
    // Pose parameters: first name wins, duplicates widen the range.
    let mut param_maps: Vec<Vec<usize>> = Vec::new();
    for m in &models {
        let mut local = Vec::new();
        for p in &m.params {
            match set.params.iter().position(|q| q.name.eq_ignore_ascii_case(&p.name)) {
                Some(i) => {
                    let q = &mut set.params[i];
                    q.start = q.start.min(p.start);
                    q.end = q.end.max(p.end);
                    local.push(i);
                }
                None => {
                    local.push(set.params.len());
                    set.params.push(p.clone());
                }
            }
        }
        param_maps.push(local);
    }
    // Sequences: first name wins; autolayers resolve by name afterwards.
    let mut seq_names: HashMap<String, usize> = HashMap::new();
    let mut pending: Vec<(usize, usize)> = Vec::new(); // (merged, model)
    for (mi, m) in models.iter().enumerate() {
        for (si, s) in m.sequences.iter().enumerate() {
            let key = s.name.to_lowercase();
            if seq_names.contains_key(&key) {
                continue;
            }
            seq_names.insert(key, set.sequences.len());
            let mut weights = vec![0.0; set.defaults.len()];
            for (w, t) in s.weights.iter().zip(&bone_maps[mi]) {
                if let Some(t) = t {
                    weights[*t] = *w;
                }
            }
            let axis = |k: usize| Axis {
                param: s.params[k].and_then(|p| param_maps[mi].get(p).copied()),
                keys: s.keys[k].clone(),
                range: s.range[k],
            };
            pending.push((set.sequences.len(), mi * 100_000 + si));
            set.sequences.push(Sequence {
                name: s.name.clone(),
                activity: s.activity.clone(),
                activity_weight: s.activity_weight,
                looping: s.flags & 0x1 != 0,
                snap: s.flags & 0x2 != 0,
                delta: s.flags & 0x4 != 0,
                post: s.flags & 0x10 != 0,
                fade_in: s.fade.0,
                fade_out: s.fade.1,
                grid: s.grid,
                anims: s
                    .anims
                    .iter()
                    .map(|a| anim_maps[mi].get(*a).copied().unwrap_or(0))
                    .collect(),
                axes: [axis(0), axis(1)],
                autolayers: Vec::new(),
                bone_weights: weights,
                events: s.events.clone(),
            });
        }
    }
    for (merged, key) in pending {
        let (mi, si) = (key / 100_000, key % 100_000);
        let m = &models[mi];
        set.sequences[merged].autolayers = m.sequences[si]
            .autolayers
            .iter()
            .filter_map(|(seq, [start, peak, tail, end])| {
                let name = m.sequences.get(*seq)?.name.to_lowercase();
                Some(AutoLayer {
                    sequence: *seq_names.get(&name)?,
                    start: *start,
                    peak: *peak,
                    tail: *tail,
                    end: *end,
                })
            })
            .collect();
    }
    set.names = seq_names;
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressed_values() {
        let data = [0x00u8, 0x80, 0x3a, 0x96, 0x00, 0x40];
        let q = quat48(&Bytes(&data), 0).unwrap();
        let want = Quat::from_xyzw(0.0, 0.173645, 0.0, 0.984808);
        assert!(Vec4::from(q).distance(Vec4::from(want)) < 1e-5, "{q}");
        let data = [0x4eu8, 0x3e, 0x96, 0xc1, 0x00, 0x00];
        let v = vec48(&Bytes(&data), 0).unwrap();
        assert!(v.distance(Vec3::new(1.576172, -2.792969, 0.0)) < 1e-5, "{v}");
    }
}
