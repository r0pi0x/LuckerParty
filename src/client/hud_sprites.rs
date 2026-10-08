//! The game HUD's sprite pieces: the ammo-type icon in the ammo panel, and
//! the damage direction indicators (the `pain_*` sprites) that flash on the
//! side a hit came from.

use bevy::prelude::*;

use crate::{
    core::{Damage, Intent, LocalPlayer},
    map::hud::{ActiveHud, GameHud, HudSprite},
    rules::Dead,
    weapon::{Inventory, Weapon},
};

pub struct HudSpritesPlugin;

impl Plugin for HudSpritesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                build.run_if(resource_exists_and_changed::<ActiveHud>),
                teardown.run_if(resource_removed::<ActiveHud>),
                (pain, update).chain().run_if(resource_exists::<ActiveHud>),
            ),
        );
    }
}

/// Seconds a damage indicator takes to fade (an assumption; the game's
/// timing isn't in its data files).
const PAIN_SECONDS: f32 = 1.0;
/// How far from the screen centre the indicators sit, in virtual units.
const PAIN_OFFSET: f32 = 40.0;

#[derive(Component)]
struct SpritePart;

#[derive(Component)]
struct AmmoIcon;

/// Up (in front), down (behind), left, right, and seconds left.
#[derive(Component)]
struct Pain(usize, f32);

const PAIN_NAMES: [&str; 4] = ["pain_up", "pain_down", "pain_left", "pain_right"];

/// The ammo icon for a weapon (CS:S ammo types by weapon).
fn ammo_icon(weapon: &str) -> Option<&'static str> {
    let short = weapon.rsplit([':', '_']).next().unwrap_or(weapon);
    Some(match short {
        "ak47" | "scout" | "g3sg1" => "ammo_762",
        "m4a1" | "famas" | "galil" | "sg552" | "aug" | "sg550" | "m249" => "ammo_556",
        "awp" => "ammo_338",
        "deagle" => "ammo_50",
        "usp" | "mac10" | "ump45" => "ammo_45",
        "glock" | "mp5navy" | "tmp" | "elite" => "ammo_9mm",
        "p90" | "fiveseven" => "ammo_57",
        "p228" => "ammo_357",
        _ => return None,
    })
}

fn image(hud: &ActiveHud, sprite: &HudSprite, color: Color) -> Option<ImageNode> {
    let handle = hud.1.get(&sprite.texture)?.clone();
    let [x, y, w, h] = sprite.rect;
    Some(ImageNode {
        image: handle,
        color,
        rect: Some(Rect::new(x, y, x + w, y + h)),
        ..default()
    })
}

/// The damage indicators' sheet as CS:S draws it, additively: black is
/// nothing. Drawn alpha blended here, so each texel's brightness becomes
/// its coverage (colour divided by it); otherwise the sprite's black
/// surround shows as a dark box. `None` if the image isn't 8-bit RGBA.
fn additive_as_alpha(image: &Image) -> Option<Image> {
    use bevy::render::render_resource::TextureFormat;
    if !matches!(
        image.texture_descriptor.format,
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb
    ) {
        return None;
    }
    let mut out = image.clone();
    let data = out.data.as_mut()?;
    for px in data.chunks_exact_mut(4) {
        let peak = px[0].max(px[1]).max(px[2]);
        if peak == 0 {
            px.copy_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        for c in &mut px[..3] {
            *c = ((*c as u32 * 255 + peak as u32 / 2) / peak as u32) as u8;
        }
        px[3] = ((px[3] as u32 * peak as u32 + 127) / 255) as u8;
    }
    Some(out)
}

fn build(
    hud: Res<ActiveHud>,
    old: Query<Entity, With<SpritePart>>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    for e in &old {
        commands.entity(e).despawn();
    }
    let absolute = || Node {
        position_type: PositionType::Absolute,
        ..default()
    };
    commands.spawn((SpritePart, AmmoIcon, absolute(), Visibility::Hidden, GlobalZIndex(40)));
    for (i, name) in PAIN_NAMES.iter().enumerate() {
        let Some(mut node) = hud.0.sprites.get(*name).and_then(|s| image(&hud, s, Color::srgba(1.0, 0.0, 0.0, 0.0)))
        else {
            continue;
        };
        if let Some(converted) = images.get(&node.image).and_then(additive_as_alpha) {
            node.image = images.add(converted);
        }
        commands.spawn((SpritePart, Pain(i, 0.0), node, absolute(), GlobalZIndex(41)));
    }
}

fn teardown(parts: Query<Entity, With<SpritePart>>, mut commands: Commands) {
    for e in &parts {
        commands.entity(e).despawn();
    }
}

/// Light the indicator on the side each hit on the local player came from.
fn pain(
    mut damage: MessageReader<Damage>,
    me: Option<Single<(Entity, &Intent, Has<crate::core::God>), With<LocalPlayer>>>,
    mut parts: Query<&mut Pain>,
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    for mut p in &mut parts {
        p.1 = (p.1 - dt).max(0.0);
    }
    let Some(me) = me else {
        damage.clear();
        return;
    };
    let (me, intent, god) = *me;
    for d in damage.read() {
        // Nothing to show when the hit takes nothing (god mode).
        if d.target != me || god {
            continue;
        }
        // A fall has no direction: every side lights (a guess; see
        // docs/tech-debt.md).
        let lit = if d.kind == crate::core::DamageKind::Fall {
            vec![0, 1, 2, 3]
        } else if d.attacker.is_none_or(|a| a == me) {
            continue;
        } else {
            sides(d.dir, intent.yaw)
        };
        for mut p in &mut parts {
            if lit.contains(&p.0) {
                p.1 = PAIN_SECONDS;
            }
        }
    }
}

/// Which indicators a hit lights (0 up/front, 1 down/behind, 2 left,
/// 3 right): from the shot's direction (`dir`, attacker toward you) and
/// your look yaw.
fn sides(dir: Vec3, yaw: f32) -> Vec<usize> {
    let from = -dir.with_y(0.0).normalize_or_zero();
    let forward = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
    let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
    let (f, r) = (from.dot(forward), from.dot(right));
    [(f > 0.3, 0), (f < -0.3, 1), (r < -0.3, 2), (r > 0.3, 3)]
        .into_iter()
        .filter_map(|(on, i)| on.then_some(i))
        .collect()
}

#[allow(clippy::type_complexity)]
fn update(
    hud: Res<ActiveHud>,
    windows: Query<&Window>,
    player: Option<Single<(Option<&Inventory>, Has<Dead>), With<LocalPlayer>>>,
    weapons: Query<&Weapon>,
    mut icon: Query<(&mut Node, &mut Visibility, Option<&mut ImageNode>, Entity), (With<AmmoIcon>, Without<Pain>)>,
    mut pains: Query<(&Pain, &mut Node, &mut ImageNode), Without<AmmoIcon>>,
    mut commands: Commands,
) {
    let Some(window) = windows.iter().next() else { return };
    let (w, h) = (window.width(), window.height());
    let scale = h / 480.0;
    let game: &GameHud = &hud.0;
    let (inventory, dead) = player.map(|p| *p).unwrap_or((None, true));
    // Ammo icon.
    if let (Ok((mut node, mut vis, image_node, e)), Some(panel)) = (icon.single_mut(), game.panels.get("HudAmmo")) {
        let sprite = inventory
            .and_then(|i| i.active)
            .and_then(|w| weapons.get(w).ok())
            .and_then(|w| ammo_icon(w.id))
            .and_then(|n| game.sprites.get(n));
        match sprite.filter(|_| !dead) {
            Some(s) => {
                let fg = game.color("FgColor").unwrap_or(Color::srgb_u8(255, 176, 0));
                let want = image(&hud, s, fg);
                match (image_node, want) {
                    (Some(mut current), Some(want)) => {
                        if current.rect != want.rect {
                            *current = want;
                        }
                    }
                    (None, Some(want)) => {
                        commands.entity(e).insert(want);
                    }
                    _ => {}
                }
                let x = panel.x.resolve(w, scale) + panel.icon.x * scale;
                let y = panel.y.resolve(h, scale) + panel.icon.y * scale;
                node.left = px(x);
                node.top = px(y);
                node.width = px(s.rect[2] * scale * 0.75);
                node.height = px(s.rect[3] * scale * 0.75);
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
    // Damage indicators around the centre.
    let centre = Vec2::new(w, h) / 2.0;
    for (p, mut node, mut img) in &mut pains {
        let Some(s) = game.sprites.get(PAIN_NAMES[p.0]) else { continue };
        let size = Vec2::new(s.rect[2], s.rect[3]) * scale * 0.6;
        let off = PAIN_OFFSET * scale;
        let at = match p.0 {
            0 => Vec2::new(centre.x - size.x / 2.0, centre.y - off - size.y),
            1 => Vec2::new(centre.x - size.x / 2.0, centre.y + off),
            2 => Vec2::new(centre.x - off - size.x, centre.y - size.y / 2.0),
            _ => Vec2::new(centre.x + off, centre.y - size.y / 2.0),
        };
        node.left = px(at.x);
        node.top = px(at.y);
        node.width = px(size.x);
        node.height = px(size.y);
        let alpha = if dead { 0.0 } else { (p.1 / PAIN_SECONDS).clamp(0.0, 1.0) };
        img.color = Color::srgba(1.0, 0.1, 0.0, alpha * 0.8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_light_the_side_they_came_from() {
        // Facing -Z (yaw 0); a shot travelling +Z came from in front.
        assert_eq!(sides(Vec3::Z, 0.0), vec![0]);
        assert_eq!(sides(Vec3::NEG_Z, 0.0), vec![1]);
        // From the left (-X side): the shot travels +X.
        assert_eq!(sides(Vec3::X, 0.0), vec![2]);
        assert_eq!(sides(Vec3::NEG_X, 0.0), vec![3]);
        // Turned 90 degrees left (facing -X), a shot from -X is in front.
        assert_eq!(sides(Vec3::X, std::f32::consts::FRAC_PI_2), vec![0]);
        // Diagonal: front and right.
        assert_eq!(sides(Vec3::new(-1.0, 0.0, 1.0), 0.0), vec![0, 3]);
    }

    #[test]
    fn additive_sprites_turn_black_into_transparency() {
        use bevy::{
            asset::RenderAssetUsages,
            render::render_resource::{Extent3d, TextureDimension, TextureFormat},
        };
        let img = Image::new(
            Extent3d { width: 3, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2,
            vec![0, 0, 0, 255, 128, 0, 0, 255, 255, 255, 255, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::all(),
        );
        let out = additive_as_alpha(&img).unwrap();
        let d = out.data.unwrap();
        assert_eq!(&d[0..4], &[0, 0, 0, 0], "black is clear");
        assert_eq!(&d[4..8], &[255, 0, 0, 128], "half-bright red is full red at half coverage");
        assert_eq!(&d[8..12], &[255, 255, 255, 255], "white stays");
    }

    #[test]
    fn ammo_icons_by_weapon() {
        assert_eq!(ammo_icon("cs_source:weapon_ak47"), Some("ammo_762"));
        assert_eq!(ammo_icon("cs_source:weapon_usp"), Some("ammo_45"));
        assert_eq!(ammo_icon("cs_source:weapon_knife"), None);
    }
}
