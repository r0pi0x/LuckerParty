//! What the bomb shows (specs/cs_source/objectives.md, HUD table): the
//! code typed on the C4's little screen while arming, drawn on its view
//! model (`map::ViewModelScreen` between the model's `controlpanel0_*`
//! attachments; the text from `BombRules::screen_text`, in the C4
//! panel's font and armed colour, `GameHud::screens["c4_view_panel"]`),
//! the planted bomb's LED glow (`sprites/ledglow`) flashing with each
//! beep, and its marker (`sprites/c4`, additive, drawn through walls as
//! its material's `$ignorez` says).
//!
//! Guesses (spec Q4, Q12; not measured): who sees the marker (the bomb's
//! team, and the dead while spectating), its size (32 units) and height
//! over the bomb (16), the LED's place on the bomb (9 units over its origin, on top), size
//! (8 units) and how long a flash lasts (`LED_FLASH`).

use bevy::{
    asset::RenderAssetUsages,
    camera::RenderTarget,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
};

use crate::{
    core::{Intent, LocalPlayer, Team},
    map::{
        ViewModelScreen,
        hud::ActiveHud,
        sprite_material::{SpriteMaterial, SpriteParams, sprite_mesh},
    },
    objectives::{
        ObjectiveEvent,
        bomb::{Arming, BombRules, PlantedBomb},
    },
    rules::Dead,
};

const UNIT: f32 = 0.0254;
/// Seconds an LED flash takes to fade out.
pub const LED_FLASH: f32 = 0.15;
const MARKER_SIZE: f32 = 32.0;
const MARKER_HEIGHT: f32 = 16.0;
const LED_SIZE: f32 = 8.0;
const LED_HEIGHT: f32 = 9.0;
/// The screen picture's pixels per panel pixel (sharper text).
const SCREEN_SCALE: f32 = 2.0;

pub struct BombFxPlugin;

impl Plugin for BombFxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (screen, planted).run_if(resource_exists::<Assets<Image>>));
    }
}

/// The picture drawn on the C4 view model's screen: a camera drawing the
/// screen's text node into an image.
#[derive(Resource)]
struct C4Screen {
    image: Handle<Image>,
    camera: Entity,
    text: Entity,
    /// The look it was made for (panel size, font, colour).
    look: (Vec2, Option<(String, f32, bool)>, [u8; 4]),
}

/// Who the first-person view belongs to: the watched player, else the
/// local one.
fn viewer(spec: Option<&super::spectate::SpecView>, local: Option<Entity>) -> Option<Entity> {
    spec.and_then(|s| s.in_eye).or(local)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn screen(
    rules: Option<Res<BombRules>>,
    hud: Option<Res<ActiveHud>>,
    fonts: Res<super::fonts::UiFonts>,
    clock: Res<crate::core::SimClock>,
    spec: Option<Res<super::spectate::SpecView>>,
    local: Query<Entity, With<LocalPlayer>>,
    characters: Query<(Entity, Option<&Arming>, Has<ViewModelScreen>), With<Intent>>,
    mut texts: Query<&mut Text>,
    mut cameras: Query<&mut Camera>,
    state: Option<ResMut<C4Screen>>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    let (Some(rules), Some(key)) = (rules.as_ref(), rules.as_ref().and_then(|r| r.weapon)) else {
        return;
    };
    let look = hud.as_ref().and_then(|h| h.0.screens.get("c4_view_panel"));
    let wanted = look.map(|l| {
        let color = l.colors.get("C4Panel_Armed").copied().unwrap_or([255, 30, 13, 200]);
        let font = l.font.as_ref().map(|f| (f.family.clone(), f.tall, f.bold()));
        (l.pixels.max(Vec2::ONE), font, color)
    });
    // (Re)make the picture for the install's look.
    let state = match (state, wanted) {
        (Some(s), Some(w)) if s.look == w => s,
        (old, Some(w)) => {
            if let Some(old) = old {
                commands.entity(old.camera).despawn();
                commands.entity(old.text).despawn();
            }
            make_screen(&mut commands, &mut images, &fonts, w);
            return;
        }
        (old, None) => {
            if let Some(old) = old {
                commands.entity(old.camera).despawn();
                commands.entity(old.text).despawn();
                commands.remove_resource::<C4Screen>();
            }
            return;
        }
    };
    let me = viewer(spec.as_deref(), local.iter().next());
    let mut text = String::new();
    for (e, arming, has) in &characters {
        let mine = Some(e) == me;
        if mine && let Some(a) = arming {
            // Arming's time is the simulation's (a network client predicts it).
            text = rules.screen_text((clock.now - a.since) as f32);
        }
        if mine && !has {
            commands.entity(e).insert(ViewModelScreen {
                key: key.to_string(),
                image: state.image.clone(),
            });
        } else if !mine && has {
            commands.entity(e).remove::<ViewModelScreen>();
        }
    }
    if let Ok(mut t) = texts.get_mut(state.text)
        && t.0 != text
    {
        t.0 = text;
    }
    // Drawn only while there is something on it.
    if let Ok(mut c) = cameras.get_mut(state.camera) {
        let active = texts.get(state.text).is_ok_and(|t| !t.0.is_empty());
        if c.is_active != active {
            c.is_active = active;
        }
    }
}

fn make_screen(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    fonts: &super::fonts::UiFonts,
    look: (Vec2, Option<(String, f32, bool)>, [u8; 4]),
) {
    let (pixels, font, [r, g, b, a]) = look.clone();
    let size = (pixels * SCREEN_SCALE).as_uvec2();
    let mut image = Image::new_fill(
        Extent3d {
            width: size.x,
            height: size.y,
            ..default()
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    let handle = images.add(image);
    let camera = commands
        .spawn((
            Name::new("C4 screen camera"),
            Camera2d,
            Camera {
                order: -10,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                is_active: false,
                ..default()
            },
            RenderTarget::Image(handle.clone().into()),
        ))
        .id();
    let (family, tall, bold) = font.unwrap_or(("Courier New".into(), 45.0, true));
    let text = commands
        .spawn((
            Node {
                width: percent(100.0),
                height: percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            UiTargetCamera(camera),
        ))
        .id();
    let label = commands
        .spawn((
            Text::new(""),
            fonts.line(&family, bold, tall * SCREEN_SCALE, true),
            TextColor(Color::srgba_u8(r, g, b, a)),
            ChildOf(text),
        ))
        .id();
    commands.insert_resource(C4Screen {
        image: handle,
        camera,
        text: label,
        look,
    });
    let _ = text;
}

/// A planted bomb's marker and LED.
struct BombSprites {
    bomb: Entity,
    marker: Entity,
    led: Entity,
    led_material: Handle<SpriteMaterial>,
    /// When it last beeped (Time).
    beep: f64,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn planted(
    time: Res<Time>,
    rules: Option<Res<BombRules>>,
    hud: Option<Res<ActiveHud>>,
    bombs: Query<(Entity, &PlantedBomb, &Transform)>,
    mut drawn: Local<Vec<BombSprites>>,
    local: Query<(Option<&Team>, Has<Dead>), With<LocalPlayer>>,
    mut events: MessageReader<ObjectiveEvent>,
    mut visibility: Query<&mut Visibility>,
    (meshes, materials): (Option<ResMut<Assets<Mesh>>>, Option<ResMut<Assets<SpriteMaterial>>>),
    mut commands: Commands,
) {
    let (Some(hud), Some(mut meshes), Some(mut materials)) = (hud, meshes, materials) else {
        events.clear();
        return;
    };
    let now = time.elapsed_secs_f64();
    let beeps: Vec<Vec3> = events
        .read()
        .filter_map(|e| match e {
            ObjectiveEvent::Beep { at } => Some(*at),
            _ => None,
        })
        .collect();
    // The marker for the bomb's team, and the dead (spectating).
    let (team, dead) = local.iter().next().map_or((None, true), |(t, d)| (t.copied(), d));
    let sees_marker = dead || rules.as_ref().is_some_and(|r| team == Some(r.carrier_team));
    let sprite = |name: &str| {
        let s = hud.0.sprites.get(name)?;
        Some((hud.1.get(&s.texture)?.clone(), UVec2::new(s.rect[2] as u32, s.rect[3] as u32)))
    };
    // Bombs gone (exploded, a new round) take theirs along.
    drawn.retain(|d| {
        let keep = bombs.contains(d.bomb);
        if !keep {
            commands.entity(d.marker).try_despawn();
            commands.entity(d.led).try_despawn();
        }
        keep
    });
    for (e, bomb, t) in &bombs {
        let Some(sprites) = drawn.iter_mut().find(|d| d.bomb == e) else {
            let (Some((marker_tex, marker_px)), Some((led_tex, led_px))) = (sprite("c4"), sprite("ledglow")) else {
                continue;
            };
            let up = Vec3::Y * UNIT;
            let marker = commands
                .spawn((
                    Name::new("Planted bomb marker"),
                    Mesh3d(meshes.add(sprite_mesh(marker_px))),
                    MeshMaterial3d(materials.add(SpriteMaterial {
                        params: SpriteParams {
                            color: Vec4::ONE,
                            size: Vec2::splat(MARKER_SIZE * UNIT),
                        },
                        texture: Some(marker_tex),
                        glow: true,
                    })),
                    bevy::light::NotShadowCaster,
                    bevy::camera::visibility::NoFrustumCulling,
                    Transform::from_translation(t.translation + up * MARKER_HEIGHT),
                    Visibility::Hidden,
                ))
                .id();
            let led_material = materials.add(SpriteMaterial {
                params: SpriteParams {
                    color: Vec4::ZERO,
                    size: Vec2::splat(LED_SIZE * UNIT),
                },
                texture: Some(led_tex),
                glow: false,
            });
            let led = commands
                .spawn((
                    Name::new("Planted bomb LED"),
                    Mesh3d(meshes.add(sprite_mesh(led_px))),
                    MeshMaterial3d(led_material.clone()),
                    bevy::light::NotShadowCaster,
                    bevy::camera::visibility::NoFrustumCulling,
                    Transform::from_translation(t.translation + up * LED_HEIGHT),
                ))
                .id();
            drawn.push(BombSprites {
                bomb: e,
                marker,
                led,
                led_material,
                beep: f64::NEG_INFINITY,
            });
            continue;
        };
        if beeps.iter().any(|at| at.distance(t.translation) < 1.0) {
            sprites.beep = now;
        }
        let glow = if bomb.defused { 0.0 } else { led_glow((now - sprites.beep) as f32) };
        if let Some(mut m) = materials.get_mut(&sprites.led_material)
            && m.params.color.w != glow
        {
            m.params.color = Vec4::splat(glow);
        }
        if let Ok(mut v) = visibility.get_mut(sprites.marker) {
            v.set_if_neq(if sees_marker && !bomb.defused {
                Visibility::Visible
            } else {
                Visibility::Hidden
            });
        }
    }
}

/// The LED's brightness `since` seconds after a beep: full, fading out
/// over `LED_FLASH`.
pub fn led_glow(since: f32) -> f32 {
    if since < 0.0 { 0.0 } else { (1.0 - since / LED_FLASH).max(0.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_led_flashes_with_each_beep_then_fades() {
        assert_eq!(led_glow(0.0), 1.0);
        assert!((led_glow(LED_FLASH / 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(led_glow(LED_FLASH), 0.0);
        assert_eq!(led_glow(1.0), 0.0);
        assert_eq!(led_glow(-1.0), 0.0);
    }
}
