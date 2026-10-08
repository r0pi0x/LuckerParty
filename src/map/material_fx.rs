//! Material effects shared by the world and prop shaders
//! (specs/cs_source/shaders.md): texture coordinate transforms
//! (`$basetexturetransform` and a TextureScroll proxy on it), the detail
//! combine modes and self-illumination. The combine and self-illum
//! functions here mirror world.wgsl and prop.wgsl on the CPU, so the
//! spec's test cases check the maths the shaders run.

use bevy::math::{Vec2, Vec3, Vec4};

/// A texture coordinate transform: uv' = rows · (u, v, 1), plus a
/// translation scrolling at `scroll` (texture repeats per second, wrapped
/// to [0, 1) as Source's TextureScroll proxy does).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapUvTransform {
    pub rows: [[f32; 3]; 2],
    pub scroll: [f32; 2],
}

impl Default for MapUvTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl MapUvTransform {
    pub const IDENTITY: Self = Self {
        rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        scroll: [0.0, 0.0],
    };

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// From the parts of a VMT transform ("center cx cy scale sx sy rotate
    /// deg translate tx ty"; the Valve Developer Wiki's
    /// `$basetexturetransform` page: scale fits the texture that many
    /// times, the centre is the point of rotation only, rotation is
    /// counter-clockwise in degrees, translate shifts by whole repeats):
    /// uv' = c + R(deg) · (S · uv − c) + t, with R = [[cos, −sin], [sin,
    /// cos]] in (u, v). The order and the sense of rotation in (u, v) are
    /// taken, not specified (shaders.md open question 16).
    pub fn from_parts(center: Vec2, scale: Vec2, rotate_degrees: f32, translate: Vec2) -> Self {
        let (s, c) = rotate_degrees.to_radians().sin_cos();
        let a = [c * scale.x, -s * scale.y];
        let b = [s * scale.x, c * scale.y];
        let rc = Vec2::new(c * center.x - s * center.y, s * center.x + c * center.y);
        let off = center - rc + translate;
        Self {
            rows: [[a[0], a[1], off.x], [b[0], b[1], off.y]],
            scroll: [0.0, 0.0],
        }
    }

    /// A VMT transform string. Keywords may come in any order and any may
    /// be missing (the default then: center .5 .5, scale 1 1, rotate 0,
    /// translate 0 0); a lone number after `scale` scales both axes. None
    /// when nothing in it is readable.
    pub fn parse(v: &str) -> Option<Self> {
        let t: Vec<&str> = v.split_whitespace().collect();
        let num = |i: usize| t.get(i).and_then(|w| w.parse::<f32>().ok());
        let (mut center, mut scale, mut rotate, mut translate) = (Vec2::splat(0.5), Vec2::ONE, 0.0, Vec2::ZERO);
        let mut any = false;
        for (i, w) in t.iter().enumerate() {
            let pair = || Some(Vec2::new(num(i + 1)?, num(i + 2).or(num(i + 1))?));
            match w.to_ascii_lowercase().as_str() {
                "center" => {
                    if let Some(p) = pair() {
                        center = p;
                        any = true;
                    }
                }
                "scale" => {
                    if let Some(p) = pair() {
                        scale = p;
                        any = true;
                    }
                }
                "rotate" => {
                    if let Some(r) = num(i + 1) {
                        rotate = r;
                        any = true;
                    }
                }
                "translate" => {
                    if let Some(p) = pair() {
                        translate = p;
                        any = true;
                    }
                }
                _ => {}
            }
        }
        any.then(|| Self::from_parts(center, scale, rotate, translate))
    }

    /// Source's TextureScroll proxy writing this transform: a translation
    /// of `rate` (repeats per second) along `angle_degrees`, after scaling
    /// by `scale` (the proxy's `texturescale`, 1 when unset). It replaces
    /// the material's own value (specs/cs_source/water.md, TextureScroll).
    pub fn scrolling(rate: f32, angle_degrees: f32, scale: f32) -> Self {
        let (s, c) = angle_degrees.to_radians().sin_cos();
        Self {
            rows: [[scale, 0.0, 0.0], [0.0, scale, 0.0]],
            scroll: [rate * c, rate * s],
        }
    }

    /// The transformed coordinates at time `t` (seconds).
    pub fn apply(&self, uv: Vec2, t: f32) -> Vec2 {
        let [a, b] = self.rows;
        let scroll = (Vec2::from_array(self.scroll) * t).fract_gl();
        Vec2::new(a[0] * uv.x + a[1] * uv.y + a[2], b[0] * uv.x + b[1] * uv.y + b[2]) + scroll
    }

    /// The two rows for a shader: (m00, m01, m02, scroll u) and (m10, m11,
    /// m12, scroll v).
    pub fn shader_rows(&self) -> [Vec4; 2] {
        let [a, b] = self.rows;
        [
            Vec4::new(a[0], a[1], a[2], self.scroll[0]),
            Vec4::new(b[0], b[1], b[2], self.scroll[1]),
        ]
    }
}

/// `$detailblendmode` and the two-texture blend's own detail modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailMode {
    /// Source's `$detailblendmode` 0-9 (shaders.md 2, "$detail").
    Source(u8),
    /// WorldTwoTextureBlend: detail over base
    /// (specs/cs_source/shaders_two_texture_blend.md).
    TwoTextureOver,
    /// WorldTwoTextureBlend's 2x grime mask.
    TwoTextureMask,
}

impl DetailMode {
    /// The shader's `detail` parameter: 0 none, 1 + the Source mode, 20
    /// and 21 the two-texture modes (see world.wgsl).
    pub fn shader_value(self) -> f32 {
        match self {
            Self::Source(m) => m as f32 + 1.0,
            Self::TwoTextureOver => 20.0,
            Self::TwoTextureMask => 21.0,
        }
    }
}

/// Source `$selfillum`: the surface glows with its own colour where the
/// mask says (base alpha, or `$selfillummask` on models).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapSelfIllum {
    /// `$selfillumtint`, linear as the shader uses it.
    pub tint: [f32; 3],
    /// `$selfillummask` (index into `MapData::textures`, linear); None:
    /// the base texture's alpha.
    pub mask: Option<usize>,
}

/// The pre-lighting detail combine (shaders.md 2, "$detail"; modes 5 and 6
/// add after lighting, see `detail_post_lighting`): albedo `a` (linear
/// rgba), detail texel `d` (as sampled, times `$detailtint`), blend factor
/// `f`. Mirrors world.wgsl and prop.wgsl.
pub fn detail_combine(mode: u8, a: Vec4, d: Vec4, f: f32) -> Vec4 {
    let rgb = a.truncate();
    let drgb = d.truncate();
    match mode {
        0 => (rgb * Vec3::ONE.lerp(2.0 * drgb, f)).extend(a.w),
        1 => (rgb + f * drgb).extend(a.w),
        2 => rgb.lerp(drgb, f * d.w).extend(a.w),
        3 => a.lerp(d, f),
        4 => rgb.lerp(drgb, f * (1.0 - a.w)).extend(d.w),
        7 => {
            let c = d.x + (d.w - d.x) * a.w;
            (rgb * (1.0 + (2.0 * c - 1.0) * f)).extend(a.w)
        }
        8 => a.lerp(a * d, f),
        9 => rgb.extend(a.w + (a.w * d.w - a.w) * f),
        _ => a,
    }
}

/// What the model shaders add after lighting for detail modes 5 and 6
/// (shaders.md 4); zero for the other modes.
pub fn detail_post_lighting(mode: u8, d: Vec3, f: f32) -> Vec3 {
    match mode {
        5 => f * d,
        6 => {
            let (m, a) = if f >= 0.5 {
                (1.0 / f, 1.0 - 1.0 / f)
            } else {
                (4.0 * f, -2.0 * f)
            };
            (m * d + Vec3::splat(a)).clamp(Vec3::ZERO, Vec3::ONE)
        }
        _ => Vec3::ZERO,
    }
}

/// Self-illumination (shaders.md 2 and 4): the lit colour toward
/// `tint` × albedo by `mask`.
pub fn self_illum(diffuse: Vec3, albedo: Vec3, tint: Vec3, mask: f32) -> Vec3 {
    diffuse.lerp(tint * albedo, mask)
}
