//! Dust mote simulation and drawing for `MapDust` volumes, per
//! specs/cs_source/sprites_dust.md: motes spawn at random points in the
//! volume, drift (horizontal speed dies out toward the wind, vertical stays),
//! fade in and out over their life and with view depth, and keep a constant
//! size on screen. One mesh per volume, rebuilt each frame facing the camera.

use bevy::{asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology};

use super::{MapDust, SkyboxCamera};

/// All dust shares this many motes at most (the game's particle limit).
const MAX_MOTES: usize = 2048;
/// Horizontal speed moves toward the wind (none) at this rate, m/s^2.
const WIND_PULL: f32 = 50.0 * 0.0254;

struct Mote {
    position: Vec3,
    velocity: Vec3,
    age: f32,
    life: f32,
    size: f32,
}

#[derive(Component)]
pub(super) struct DustEmitter {
    pub dust: MapDust,
    pub mesh: Handle<Mesh>,
    motes: Vec<Mote>,
    pending: f32,
    rng: u64,
}

impl DustEmitter {
    pub fn new(dust: MapDust, mesh: Handle<Mesh>, seed: u64) -> Self {
        Self {
            dust,
            mesh,
            motes: Vec::new(),
            pending: 0.0,
            rng: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
        }
    }

    /// Uniform in 0..1 (xorshift).
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    fn between(&mut self, (a, b): (f32, f32)) -> f32 {
        a + (b - a) * self.random()
    }
}

/// A mesh for an emitter to fill each frame: one invisible triangle, since
/// Bevy's mesh allocator logs a use-after-free for meshes with no vertices
/// (it skips allocating them but still copies their data).
pub(super) fn empty_mesh() -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 3]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; 3]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 3]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 3]);
    mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
    mesh
}

#[allow(clippy::type_complexity)]
pub(super) fn update_dust(
    time: Res<Time>,
    cameras: Query<&GlobalTransform, (With<Camera3d>, Without<SkyboxCamera>, Without<super::ViewModelCamera>, Without<super::water::WaterReflectionCamera>)>,
    mut emitters: Query<&mut DustEmitter>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let dt = time.delta_secs();
    let Some(eye) = cameras.iter().next() else { return };
    let (from, forward, right, up) = (
        eye.translation(),
        eye.forward().as_vec3(),
        eye.right().as_vec3(),
        eye.up().as_vec3(),
    );
    let mut total: usize = emitters.iter().map(|e| e.motes.len()).sum();
    for mut e in &mut emitters {
        // Spawn (frame time for spawning is clamped to 0.1 s).
        e.pending += e.dust.rate * dt.min(0.1);
        while e.pending >= 1.0 {
            e.pending -= 1.0;
            if total >= MAX_MOTES {
                continue;
            }
            let (min, max) = (e.dust.min, e.dust.max);
            let position = Vec3::new(
                min.x + (max.x - min.x) * e.random(),
                min.y + (max.y - min.y) * e.random(),
                min.z + (max.z - min.z) * e.random(),
            );
            let speed = e.dust.speed;
            let velocity = Vec3::new(
                (e.random() * 2.0 - 1.0) * speed,
                (e.random() * 2.0 - 1.0) * speed,
                (e.random() * 2.0 - 1.0) * speed,
            );
            let (life_range, size_range) = (e.dust.life, e.dust.size);
            let life = e.between(life_range).max(0.01);
            let size = e.between(size_range);
            e.motes.push(Mote {
                position,
                velocity,
                age: 0.0,
                life,
                size,
            });
            total += 1;
        }
        // Move and age.
        let pull = WIND_PULL * dt;
        e.motes.retain_mut(|m| {
            let horizontal = Vec2::new(m.velocity.x, m.velocity.z);
            let slowed = horizontal.clamp_length_max((horizontal.length() - pull).max(0.0));
            m.velocity.x = slowed.x;
            m.velocity.z = slowed.y;
            m.position += m.velocity * dt;
            m.age += dt;
            m.age < m.life
        });
        // Draw: camera-facing quads, far to near.
        let fade = e.dust.fade_distance;
        let mut visible: Vec<(f32, &Mote)> = e
            .motes
            .iter()
            .map(|m| ((m.position - from).dot(forward), m))
            .filter(|(depth, _)| *depth > 0.0 && *depth < fade)
            .collect();
        visible.sort_by(|a, b| b.0.total_cmp(&a.0));
        let color = Color::srgb(e.dust.color[0], e.dust.color[1], e.dust.color[2]).to_linear();
        let (mut positions, mut uvs, mut colors, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (depth, m) in visible {
            let life_alpha = 0.5 - 0.5 * (std::f32::consts::TAU * m.age / m.life).cos();
            let alpha = life_alpha * (1.0 - depth / fade) * e.dust.color[3];
            let half = m.size / 10000.0 * depth;
            let base = positions.len() as u32;
            for (cx, cy, u, v) in [
                (-1.0, -1.0, 0.0, 1.0),
                (-1.0, 1.0, 0.0, 0.0),
                (1.0, 1.0, 1.0, 0.0),
                (1.0, -1.0, 1.0, 1.0),
            ] {
                positions.push((m.position + (right * cx + up * cy) * half).to_array());
                uvs.push([u, v]);
                colors.push([color.red, color.green, color.blue, alpha]);
            }
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        if positions.is_empty() {
            // Bevy's mesh allocator rejects updates to an empty mesh (it logs
            // a use-after-free every frame): keep one invisible triangle.
            positions.extend([[0.0; 3]; 3]);
            uvs.extend([[0.0; 2]; 3]);
            colors.extend([[0.0; 4]; 3]);
            indices.extend([0, 1, 2]);
        }
        if let Some(mut mesh) = meshes.get_mut(&e.mesh) {
            let n = positions.len();
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![(-forward).to_array(); n]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
            mesh.insert_indices(Indices::U32(indices));
        }
    }
}
