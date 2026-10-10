//! Entities' render colour, alpha and mode (Source "rendercolor",
//! "renderamt", "rendermode"; the Color and Alpha inputs; AddOutput of
//! those keys): a brush entity node's meshes, and a character's body and
//! what it holds, draw with their material's colour multiplied by the
//! colour and, in a translucent render mode, faded or added by the
//! alpha. The logic layer writes `BrushTint` on mover nodes and
//! `RenderLook` on characters; a tinted node draws its own meshes, not
//! the merged chunks (`merge`).

use bevy::prelude::*;

use super::{MapAlpha, prop_material::PropMaterial, world_material::WorldMaterial};

/// Source render modes that matter here.
pub const RENDER_NORMAL: u8 = 0;
pub const RENDER_TRANS_ADD: u8 = 5;
pub const RENDER_NONE: u8 = 10;

/// How an entity is drawn: colour (sRGB 0-255), alpha (renderamt) and
/// render mode.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderLook {
    pub color: [u8; 3],
    pub alpha: u8,
    pub mode: u8,
}

impl Default for RenderLook {
    fn default() -> Self {
        Self {
            color: [255; 3],
            alpha: 255,
            mode: RENDER_NORMAL,
        }
    }
}

impl RenderLook {
    /// Drawn as its materials are.
    pub fn is_plain(&self) -> bool {
        self.color == [255; 3] && self.blend().is_none()
    }

    /// The blend its render mode asks for, with the opacity (0-1): normal
    /// mode ignores renderamt; colour, texture and alpha modes (1, 2, 4)
    /// blend by it; glow and additive modes (3, 5, 7, 8, 9) add, scaled by
    /// it; environmental (6) and none (10) aren't drawn.
    pub fn blend(&self) -> Option<(MapAlpha, f32)> {
        let a = self.alpha as f32 / 255.0;
        match self.mode {
            RENDER_NORMAL => None,
            1 | 2 | 4 => (self.alpha < 255).then_some((MapAlpha::Blend, a)),
            3 | 5 | 7 | 8 | 9 => Some((MapAlpha::Add, a)),
            6 | RENDER_NONE => Some((MapAlpha::Blend, 0.0)),
            _ => None,
        }
    }

    /// The colour multiplier (linear rgb; the game multiplies in gamma
    /// space, so the colour is converted as sRGB), with the opacity in w;
    /// an added look takes the opacity into its colour.
    pub fn factor(&self) -> Vec4 {
        let c = Color::srgb_u8(self.color[0], self.color[1], self.color[2]).to_linear();
        let rgb = Vec3::new(c.red, c.green, c.blue);
        match self.blend() {
            Some((MapAlpha::Add, a)) => (rgb * a).extend(1.0),
            Some((_, a)) => rgb.extend(a),
            None => rgb.extend(1.0),
        }
    }

    /// Not drawn at all.
    pub fn invisible(&self) -> bool {
        matches!(self.blend(), Some((MapAlpha::Blend, a)) if a <= 0.0)
    }
}

/// A brush entity node's look (white, normal: as the material is).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrushTint(pub RenderLook);

impl BrushTint {
    pub fn is_white(&self) -> bool {
        self.0.is_plain()
    }
}

/// A tinted mesh's own material, to go back to (white) or tint again.
#[derive(Component, Clone, Debug)]
pub(super) struct UntintedWorld(Handle<WorldMaterial>);

#[derive(Component, Clone, Debug)]
pub(super) struct UntintedStandard(Handle<StandardMaterial>);

/// A world material with a look applied.
fn looked_world(mut m: WorldMaterial, look: &RenderLook) -> WorldMaterial {
    let f = look.factor();
    m.params.base_color *= f;
    if let Some((blend, _)) = look.blend() {
        m.params.translucent = blend.shader_mode();
        m.alpha_mode = blend.shader_alpha_mode();
    }
    m
}

/// Nodes whose tint changed: their meshes get a tinted copy of their
/// material (or their own back).
#[allow(clippy::type_complexity)]
pub(super) fn apply_brush_tints(
    nodes: Query<(&BrushTint, &Children), Changed<BrushTint>>,
    mut world_parts: Query<(&mut MeshMaterial3d<WorldMaterial>, Option<&UntintedWorld>)>,
    mut standard_parts: Query<(&mut MeshMaterial3d<StandardMaterial>, Option<&UntintedStandard>)>,
    world_materials: Option<ResMut<Assets<WorldMaterial>>>,
    standard_materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut commands: Commands,
) {
    let (Some(mut wm), Some(mut sm)) = (world_materials, standard_materials) else {
        return;
    };
    for (tint, children) in &nodes {
        let look = tint.0;
        let f = look.factor();
        for child in children.iter() {
            if let Ok((mut mat, own)) = world_parts.get_mut(child) {
                let own = match own {
                    Some(o) => o.0.clone(),
                    None => {
                        commands.entity(child).insert(UntintedWorld(mat.0.clone()));
                        mat.0.clone()
                    }
                };
                if tint.is_white() {
                    mat.0 = own;
                } else if let Some(m) = wm.get(&own).cloned() {
                    mat.0 = wm.add(looked_world(m, &look));
                }
            } else if let Ok((mut mat, own)) = standard_parts.get_mut(child) {
                let own = match own {
                    Some(o) => o.0.clone(),
                    None => {
                        commands.entity(child).insert(UntintedStandard(mat.0.clone()));
                        mat.0.clone()
                    }
                };
                if tint.is_white() {
                    mat.0 = own;
                } else if let Some(mut m) = sm.get(&own).cloned() {
                    let c = m.base_color.to_linear();
                    m.base_color = Color::linear_rgba(c.red * f.x, c.green * f.y, c.blue * f.z, c.alpha * f.w);
                    if let Some((blend, _)) = look.blend() {
                        m.alpha_mode = match blend {
                            MapAlpha::Add => AlphaMode::Add,
                            _ => AlphaMode::Blend,
                        };
                    }
                    mat.0 = sm.add(m);
                }
            }
        }
    }
}

/// A model's own materials as they were before a look (colour, blend).
#[derive(Component, Clone, Debug)]
pub(super) struct UnlookedModel {
    base: Vec<(Vec4, f32, AlphaMode)>,
    applied: RenderLook,
}

/// Characters' looks (`RenderLook` on the character): the body and the
/// model it holds (each `probe_lit::ProbeLit` among its descendants, with
/// its own material instances) take the colour and fade or add by the
/// alpha; restored when plain again.
pub(super) fn apply_character_looks(
    characters: Query<(Entity, &RenderLook)>,
    children: Query<&Children>,
    models: Query<(&super::probe_lit::ProbeLit, Option<&UnlookedModel>)>,
    materials: Option<ResMut<Assets<PropMaterial>>>,
    mut commands: Commands,
) {
    let Some(mut materials) = materials else { return };
    for (character, look) in &characters {
        for model in children.iter_descendants(character) {
            let Ok((lit, unlooked)) = models.get(model) else {
                continue;
            };
            match unlooked {
                Some(u) if u.applied == *look => continue,
                None if look.is_plain() => continue,
                _ => {}
            }
            let base: Vec<(Vec4, f32, AlphaMode)> = match unlooked {
                Some(u) => u.base.clone(),
                None => lit
                    .materials
                    .iter()
                    .map(|h| {
                        materials.get(h).map_or((Vec4::ONE, 0.0, AlphaMode::Opaque), |m| {
                            (m.params.base_color, m.params.translucent, m.alpha_mode)
                        })
                    })
                    .collect(),
            };
            let f = look.factor();
            for (h, (color, translucent, mode)) in lit.materials.iter().zip(&base) {
                let Some(mut m) = materials.get_mut(h) else { continue };
                m.params.base_color = *color * f;
                match look.blend() {
                    Some((blend, _)) => {
                        m.params.translucent = blend.shader_mode();
                        m.alpha_mode = blend.shader_alpha_mode();
                    }
                    None => {
                        m.params.translucent = *translucent;
                        m.alpha_mode = *mode;
                    }
                }
            }
            commands.entity(model).insert(UnlookedModel { base, applied: *look });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_modes() {
        let look = |mode, alpha| RenderLook {
            color: [255; 3],
            alpha,
            mode,
        };
        // Normal mode ignores renderamt.
        assert!(look(0, 0).is_plain());
        assert_eq!(look(1, 100).blend(), Some((MapAlpha::Blend, 100.0 / 255.0)));
        assert!(look(1, 255).is_plain());
        assert_eq!(look(5, 255).blend(), Some((MapAlpha::Add, 1.0)));
        assert!(look(10, 255).invisible());
        assert!(look(1, 0).invisible());
        // Added: the opacity scales the colour.
        let f = look(5, 51).factor();
        assert!((f.x - 0.2).abs() < 1e-5 && f.w == 1.0);
        let red = RenderLook {
            color: [255, 0, 0],
            alpha: 255,
            mode: 0,
        };
        assert!(!red.is_plain());
        assert_eq!(red.factor(), Vec4::new(1.0, 0.0, 0.0, 1.0));
    }
}
