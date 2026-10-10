//! Detail props: the grass, weeds and small bushes vbsp scatters over
//! displacements and floors from the map's detail.vbsp (the BSP's `dprp`
//! game lump; public BSP description, "Detail props", and the public
//! env_detail_controller / detail.vbsp pages). Sprites and the "cross" and
//! "tri" sprite shapes become `MapDetailProps` quads (`map::detail` draws
//! them); model entries become static props without collision.
//!
//! The lump (version 4): a model dictionary (count, then 128-byte names),
//! a sprite dictionary (count, then per sprite its quad's upper-left and
//! lower-right corners in units, x across and y up, and the same corners'
//! texture coordinates), then the objects (count, 52 bytes each): origin,
//! angles, model or sprite index, leaf, lighting (`ColorRGBExp32`), light
//! styles and their count, sway amount, shape angle, shape size,
//! orientation, three pad bytes, type, three pad bytes, scale.
//!
//! How the sprite shapes are laid out and how sway moves them is our
//! reading of those pages (docs/tech-debt.md): not measured against CS:S.

use bevy::prelude::*;

use super::bsp::{METERS_PER_UNIT, to_engine};
use crate::map::{DetailQuad, MapDetailProps};

/// The game lump directory entry: id, flags, version, offset, length.
const GAME_LUMP: usize = 35;
const OBJECT_SIZE: usize = 52;
/// Object types.
const TYPE_MODEL: u8 = 0;
const TYPE_SPRITE: u8 = 1;
const TYPE_CROSS: u8 = 2;
const TYPE_TRI: u8 = 3;
/// Sprite orientations: fixed by its angles, facing the view, turning
/// only about the vertical to face it.
const ORIENT_SCREEN: u8 = 1;
const ORIENT_SCREEN_VERTICAL: u8 = 2;

/// `cl_detaildist` and `cl_detailfade` defaults (units): with no
/// env_detail_controller, detail props fade out over the last
/// `cl_detailfade` units before `cl_detaildist`.
pub const DETAIL_DIST: f32 = 1200.0;
pub const DETAIL_FADE: f32 = 400.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteDict {
    /// Upper-left and lower-right corners, units (x across, y up).
    pub ul: Vec2,
    pub lr: Vec2,
    /// Their texture coordinates.
    pub tex_ul: Vec2,
    pub tex_lr: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetailObject {
    /// Source units and degrees.
    pub origin: Vec3,
    pub angles: Vec3,
    /// Model or sprite dictionary index.
    pub index: u16,
    /// Linear lightmap units.
    pub light: [f32; 3],
    pub sway: u8,
    pub shape_angle: u8,
    pub shape_size: u8,
    pub orientation: u8,
    pub kind: u8,
    pub scale: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DetailLump {
    pub models: Vec<String>,
    pub sprites: Vec<SpriteDict>,
    pub objects: Vec<DetailObject>,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn f32_at(b: &[u8], at: usize) -> Option<f32> {
    u32_at(b, at).map(f32::from_bits)
}

fn v2(b: &[u8], at: usize) -> Option<Vec2> {
    Some(Vec2::new(f32_at(b, at)?, f32_at(b, at + 4)?))
}

fn v3(b: &[u8], at: usize) -> Option<Vec3> {
    Some(Vec3::new(f32_at(b, at)?, f32_at(b, at + 4)?, f32_at(b, at + 8)?))
}

/// The `dprp` game lump's bytes (decompressed when the map stores it
/// LZMA-compressed) and its version, from a BSP file whose own lumps are
/// plain (`lumps::inflate`). Its sub-lumps carry absolute file offsets.
fn detail_lump_bytes(bsp: &[u8]) -> Option<(Vec<u8>, u16)> {
    let at = 8 + GAME_LUMP * 16;
    let ofs = u32_at(bsp, at)? as usize;
    let len = u32_at(bsp, at + 4)? as usize;
    let lump = bsp.get(ofs..ofs + len)?;
    let count = u32_at(lump, 0)? as usize;
    let entry = |i: usize| -> Option<(u32, u16, u16, usize, usize)> {
        let e = lump.get(4 + i * 16..4 + i * 16 + 16)?;
        Some((
            u32::from_le_bytes(e[0..4].try_into().ok()?),
            u16::from_le_bytes(e[4..6].try_into().ok()?),
            u16::from_le_bytes(e[6..8].try_into().ok()?),
            u32::from_le_bytes(e[8..12].try_into().ok()?) as usize,
            u32::from_le_bytes(e[12..16].try_into().ok()?) as usize,
        ))
    };
    for i in 0..count.min(64) {
        let (id, flags, version, ofs, len) = entry(i)?;
        if id != u32::from_be_bytes(*b"dprp") {
            continue;
        }
        if flags & 1 != 0 {
            // Compressed: its data runs to the next sub-lump's offset.
            let end = entry(i + 1).map(|e| e.3)?;
            let raw = bsp.get(ofs..end)?;
            return Some((super::lumps::decompress(raw)?, version));
        }
        return Some((bsp.get(ofs..ofs + len)?.to_vec(), version));
    }
    None
}

/// The detail lump of a map, if it has one this reader knows (version 4,
/// CS:S's).
pub fn read(bsp: &[u8]) -> Option<DetailLump> {
    let (b, version) = detail_lump_bytes(bsp)?;
    if version != 4 {
        return None;
    }
    let mut out = DetailLump::default();
    let mut p = 0;
    let models = u32_at(&b, p)? as usize;
    p += 4;
    for k in 0..models.min(4096) {
        let name = b.get(p + k * 128..p + k * 128 + 128)?;
        let end = name.iter().position(|&c| c == 0).unwrap_or(128);
        out.models
            .push(String::from_utf8_lossy(&name[..end]).to_lowercase().replace('\\', "/"));
    }
    p += models * 128;
    let sprites = u32_at(&b, p)? as usize;
    p += 4;
    for k in 0..sprites.min(4096) {
        let s = p + k * 32;
        out.sprites.push(SpriteDict {
            ul: v2(&b, s)?,
            lr: v2(&b, s + 8)?,
            tex_ul: v2(&b, s + 16)?,
            tex_lr: v2(&b, s + 24)?,
        });
    }
    p += sprites * 32;
    let objects = u32_at(&b, p)? as usize;
    p += 4;
    for k in 0..objects.min(1 << 20) {
        let o = p + k * OBJECT_SIZE;
        let rgbe = b.get(o + 28..o + 32)?;
        let scale = 2f32.powi(rgbe[3] as i8 as i32) / 255.0;
        out.objects.push(DetailObject {
            origin: v3(&b, o)?,
            angles: v3(&b, o + 12)?,
            index: u16::from_le_bytes(b.get(o + 24..o + 26)?.try_into().ok()?),
            light: [rgbe[0] as f32 * scale, rgbe[1] as f32 * scale, rgbe[2] as f32 * scale],
            sway: *b.get(o + 37)?,
            shape_angle: *b.get(o + 38)?,
            shape_size: *b.get(o + 39)?,
            orientation: *b.get(o + 40)?,
            kind: *b.get(o + 44)?,
            scale: f32_at(&b, o + 48)?,
        });
    }
    Some(out)
}

/// The map's env_detail_controller fade (start, end), units: its
/// `fademindist` and `fademaxdist`; a negative start means the end less
/// `cl_detailfade` (public entity docs). None without one.
pub fn controller_fade(bsp: &vbsp::Bsp) -> Option<(f32, f32)> {
    let e = bsp
        .entities
        .iter()
        .find(|e| e.prop("classname") == Some("env_detail_controller"))?;
    let num = |k: &'static str, d: f32| e.prop(k).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(d);
    let end = num("fademaxdist", DETAIL_DIST);
    let start = num("fademindist", end - DETAIL_FADE);
    let start = if start < 0.0 { end - DETAIL_FADE } else { start };
    Some((start.clamp(0.0, end), end))
}

/// The model entries: (model path, origin, angles), Source units and
/// degrees; drawn as static props without collision.
pub fn model_entries(lump: &DetailLump) -> Vec<(String, Vec3, Vec3)> {
    lump.objects
        .iter()
        .filter(|o| o.kind == TYPE_MODEL)
        .filter_map(|o| Some((lump.models.get(o.index as usize)?.clone(), o.origin, o.angles)))
        .collect()
}

/// Source rotation of an object's angles (pitch, yaw, roll) in Source
/// axes.
fn source_rotation(angles: Vec3) -> Quat {
    Quat::from_rotation_z(angles.y.to_radians())
        * Quat::from_rotation_y(angles.x.to_radians())
        * Quat::from_rotation_x(angles.z.to_radians())
}

fn engine(v: Vec3) -> Vec3 {
    to_engine(vbsp::Vector { x: v.x, y: v.y, z: v.z })
}

/// One blade of a sprite: its quad in the object's own frame (x forward,
/// y left, z up; the blade spans y and z, facing x), Source units, as
/// [bottom-left, top-left, top-right, bottom-right] seen from the front,
/// plus how far along its height each corner is (0 bottom, 1 top).
fn blade(s: &SpriteDict, scale: f32) -> [Vec3; 4] {
    let (x0, x1) = (s.ul.x * scale, s.lr.x * scale);
    let (y0, y1) = (s.lr.y * scale, s.ul.y * scale);
    // Across the blade is -y (its right seen from the front), up is z.
    [
        Vec3::new(0.0, -x0, y0),
        Vec3::new(0.0, -x0, y1),
        Vec3::new(0.0, -x1, y1),
        Vec3::new(0.0, -x1, y0),
    ]
}

/// The quads of every sprite, cross and tri entry (engine space), with the
/// map's detail texture.
pub fn quads(lump: &DetailLump) -> Vec<DetailQuad> {
    let mut out = Vec::new();
    for o in &lump.objects {
        let Some(s) = lump.sprites.get(o.index as usize) else {
            continue;
        };
        let scale = if o.scale > 0.0 { o.scale } else { 1.0 };
        let uv = [
            Vec2::new(s.tex_ul.x, s.tex_lr.y),
            s.tex_ul,
            Vec2::new(s.tex_lr.x, s.tex_ul.y),
            s.tex_lr,
        ];
        let origin = engine(o.origin);
        let sway = o.sway as f32 / 255.0;
        let push = |out: &mut Vec<DetailQuad>, corners: [Vec3; 4], rot: Quat| {
            out.push(DetailQuad {
                origin,
                corners: corners.map(|c| engine(rot * c)),
                billboard: None,
                uv,
                light: o.light,
                sway,
            });
        };
        let rot = source_rotation(o.angles);
        match o.kind {
            TYPE_SPRITE if matches!(o.orientation, ORIENT_SCREEN | ORIENT_SCREEN_VERTICAL) => {
                let (x0, x1) = (s.ul.x * scale, s.lr.x * scale);
                let (y0, y1) = (s.lr.y * scale, s.ul.y * scale);
                out.push(DetailQuad {
                    origin,
                    corners: [Vec3::ZERO; 4],
                    billboard: Some(crate::map::DetailBillboard {
                        vertical: o.orientation == ORIENT_SCREEN_VERTICAL,
                        lo: Vec2::new(x0, y0) * METERS_PER_UNIT,
                        hi: Vec2::new(x1, y1) * METERS_PER_UNIT,
                    }),
                    uv,
                    light: o.light,
                    sway,
                });
            }
            TYPE_SPRITE => push(&mut out, blade(s, scale), rot),
            TYPE_CROSS => {
                // Two blades crossing at the origin, square to each other.
                for k in 0..2 {
                    let turn = Quat::from_rotation_z(k as f32 * std::f32::consts::FRAC_PI_2);
                    push(&mut out, blade(s, scale).map(|c| turn * c), rot);
                }
            }
            TYPE_TRI => {
                // Three blades around the origin, a third of a turn apart,
                // each moved out from the centre by its size share of the
                // blade's width and leaning out by the shape angle.
                let width = (s.lr.x - s.ul.x).abs() * scale;
                let out_by = o.shape_size as f32 / 255.0 * width;
                let lean = Quat::from_rotation_y((o.shape_angle as f32).to_radians());
                for k in 0..3 {
                    let turn = Quat::from_rotation_z(k as f32 * std::f32::consts::TAU / 3.0);
                    let corners = blade(s, scale).map(|c| turn * (lean * c + Vec3::X * out_by));
                    push(&mut out, corners, rot);
                }
            }
            _ => {}
        }
    }
    out
}

/// The map's detail sprites as `MapDetailProps` (None without any or
/// without its sprite material), with the controller's fade and env_wind's
/// sway; `texture` resolves the material to a texture index.
pub fn detail_props(
    lump: &DetailLump,
    bsp: &vbsp::Bsp,
    texture: impl FnOnce(&str) -> Option<usize>,
) -> Option<MapDetailProps> {
    let quads = quads(lump);
    if quads.is_empty() {
        return None;
    }
    let material = bsp
        .entities
        .iter()
        .find(|e| e.prop("classname") == Some("worldspawn"))
        .and_then(|e| e.prop("detailmaterial"))
        .map(|m| m.trim().to_lowercase().replace('\\', "/"))
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "detail/detailsprites".to_string());
    let texture = texture(&material)?;
    let fade = controller_fade(bsp).map(|(a, b)| (a * METERS_PER_UNIT, b * METERS_PER_UNIT));
    let wind = bsp
        .entities
        .iter()
        .find(|e| e.prop("classname") == Some("env_wind"))
        .map(|e| {
            let num = |k: &'static str| e.prop(k).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
            let yaw = e
                .prop("angles")
                .and_then(|a| a.split_whitespace().nth(1).and_then(|y| y.parse::<f32>().ok()))
                .unwrap_or(0.0);
            let dir = engine(Vec3::new(yaw.to_radians().cos(), yaw.to_radians().sin(), 0.0)).normalize_or_zero();
            (dir, (num("minwind") + num("maxwind")) / 2.0 * METERS_PER_UNIT)
        });
    Some(MapDetailProps {
        texture,
        quads,
        fade,
        wind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lump_bytes(objects: &[(u8, u8, u16)]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&1u32.to_le_bytes());
        let mut name = b"models/props/bush.mdl".to_vec();
        name.resize(128, 0);
        b.extend_from_slice(&name);
        b.extend_from_slice(&1u32.to_le_bytes());
        for f in [-6.0f32, 12.0, 6.0, 0.0, 0.5, 0.25, 0.75, 0.5] {
            b.extend_from_slice(&f.to_le_bytes());
        }
        b.extend_from_slice(&(objects.len() as u32).to_le_bytes());
        for &(kind, orientation, index) in objects {
            let mut o = Vec::new();
            for f in [10.0f32, 20.0, 30.0, 0.0, 90.0, 0.0] {
                o.extend_from_slice(&f.to_le_bytes());
            }
            o.extend_from_slice(&index.to_le_bytes());
            o.extend_from_slice(&7u16.to_le_bytes());
            o.extend_from_slice(&[128, 64, 32, 1]);
            o.extend_from_slice(&0u32.to_le_bytes());
            o.extend_from_slice(&[0, 127, 30, 25, orientation, 0, 0, 0, kind, 0, 0, 0]);
            o.extend_from_slice(&2.0f32.to_le_bytes());
            assert_eq!(o.len(), OBJECT_SIZE);
            b.extend_from_slice(&o);
        }
        b
    }

    /// A BSP holding only a game lump with `dprp`.
    fn bsp_with(dprp: &[u8]) -> Vec<u8> {
        let header = 8 + 64 * 16 + 4;
        let mut file = vec![0u8; header];
        let game_at = file.len();
        let sub = game_at + 4 + 16;
        file.extend_from_slice(&1u32.to_le_bytes());
        file.extend_from_slice(b"prpd");
        file.extend_from_slice(&0u16.to_le_bytes());
        file.extend_from_slice(&4u16.to_le_bytes());
        file.extend_from_slice(&(sub as u32).to_le_bytes());
        file.extend_from_slice(&(dprp.len() as u32).to_le_bytes());
        file.extend_from_slice(dprp);
        let at = 8 + GAME_LUMP * 16;
        file[at..at + 4].copy_from_slice(&(game_at as u32).to_le_bytes());
        let len = (file.len() - game_at) as u32;
        file[at + 4..at + 8].copy_from_slice(&len.to_le_bytes());
        file
    }

    #[test]
    fn reads_the_detail_lump() {
        let file = bsp_with(&lump_bytes(&[
            (TYPE_MODEL, 0, 0),
            (TYPE_SPRITE, ORIENT_SCREEN_VERTICAL, 0),
        ]));
        let lump = read(&file).expect("detail lump");
        assert_eq!(lump.models, ["models/props/bush.mdl"]);
        assert_eq!(lump.sprites.len(), 1);
        assert_eq!(lump.sprites[0].ul, Vec2::new(-6.0, 12.0));
        assert_eq!(lump.objects.len(), 2);
        let o = lump.objects[1];
        assert_eq!(o.origin, Vec3::new(10.0, 20.0, 30.0));
        assert_eq!(o.angles.y, 90.0);
        assert_eq!(
            (o.kind, o.orientation, o.sway, o.shape_angle, o.shape_size),
            (1, 2, 127, 30, 25)
        );
        assert_eq!(o.scale, 2.0);
        assert!((o.light[0] - 128.0 * 2.0 / 255.0).abs() < 1e-6);
        assert_eq!(model_entries(&lump).len(), 1);
    }

    #[test]
    fn sprite_shapes_make_their_blades() {
        let file = bsp_with(&lump_bytes(&[
            (TYPE_SPRITE, ORIENT_SCREEN_VERTICAL, 0),
            (TYPE_SPRITE, 0, 0),
            (TYPE_CROSS, 0, 0),
            (TYPE_TRI, 0, 0),
        ]));
        let q = quads(&read(&file).unwrap());
        assert_eq!(q.len(), 1 + 1 + 2 + 3);
        // The facing sprite: its quad in meters, scale 2, from 0 up to 24
        // units and 12 either side.
        let b = q[0].billboard.unwrap();
        assert!(b.vertical);
        assert!((b.hi.y - 24.0 * METERS_PER_UNIT).abs() < 1e-6);
        assert!((b.lo.x + 12.0 * METERS_PER_UNIT).abs() < 1e-6);
        // The fixed sprite stands up from its origin: two corners at its
        // height, two at its foot.
        let ys: Vec<f32> = q[1].corners.iter().map(|c| c.y / METERS_PER_UNIT).collect();
        assert!((ys[1] - 24.0).abs() < 1e-3 && ys[0].abs() < 1e-3, "{ys:?}");
        // The tri's blades sit out from the centre and lean out.
        for t in &q[4..] {
            let foot = (t.corners[0] + t.corners[3]) / 2.0;
            let top = (t.corners[1] + t.corners[2]) / 2.0;
            assert!(foot.xz().length() > 0.0);
            assert!(top.xz().length() > foot.xz().length(), "leans out");
        }
        // Texture corners: bottom-left takes (u of the upper left, v of
        // the lower right).
        assert_eq!(q[1].uv[0], Vec2::new(0.5, 0.5));
        assert_eq!(q[1].uv[1], Vec2::new(0.5, 0.25));
    }
}
