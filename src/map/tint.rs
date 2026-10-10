//! Brush entities' render colour (Source "rendercolor" and the Color
//! input): a node's meshes draw with their material's colour multiplied
//! by it. The logic layer writes `BrushTint` on mover nodes; a tinted
//! node draws its own meshes, not the merged chunks (`merge`).

use bevy::prelude::*;

use super::world_material::WorldMaterial;

/// A brush entity node's colour (255 255 255: as the material is).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrushTint(pub [u8; 3]);

impl BrushTint {
    pub fn is_white(&self) -> bool {
        self.0 == [255; 3]
    }

    /// The multiplier for a material colour (linear; the game multiplies
    /// in gamma space, so the colour is converted as sRGB).
    fn factor(&self) -> Vec4 {
        let c = Color::srgb_u8(self.0[0], self.0[1], self.0[2]).to_linear();
        Vec4::new(c.red, c.green, c.blue, 1.0)
    }
}

/// A tinted mesh's own material, to go back to (white) or tint again.
#[derive(Component, Clone, Debug)]
pub(super) struct UntintedWorld(Handle<WorldMaterial>);

#[derive(Component, Clone, Debug)]
pub(super) struct UntintedStandard(Handle<StandardMaterial>);

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
        let f = tint.factor();
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
                } else if let Some(mut m) = wm.get(&own).cloned() {
                    m.params.base_color *= f;
                    mat.0 = wm.add(m);
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
                    m.base_color = Color::linear_rgba(c.red * f.x, c.green * f.y, c.blue * f.z, c.alpha);
                    mat.0 = sm.add(m);
                }
            }
        }
    }
}
