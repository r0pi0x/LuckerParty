//! What the logic switches on real CS:S maps, headless: cs_office's
//! projector glow sprites (killed) and de_prodigy's bomb-site sprites and
//! lights; de_nuke's computer screens (Skin), cars (SetAnimation) and a
//! removed prop that stops blocking; de_prodigy's switchable light styles
//! in the lightmap; de_aztec's soundscape triggers through the logic's
//! touch code. Skipped without an install.

use avian3d::prelude::ColliderDisabled;
use bevy::prelude::*;
use mashup::{
    core::{LocalPlayer, MapBrushes, MovingSolid, RoundRestarts},
    games::{
        self, cs_source,
        cs_source::movement::{SourceMovementPlugin, to_engine},
    },
    harness::Sim,
    logic::{Logic, Value},
    map::{LightStyles, MapData, MapPlugin, PropEntity, PropLook, PropSequence, SoundscapeTouches},
    mount::config::LocalConfig,
    movement::noclip,
};

fn load(name: &str) -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()?
        .game_path(cs_source::GAME)
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    Some(games::load_map(&format!("cs_source:{name}")).expect("load map"))
}

fn indices(map: &MapData, name: &str) -> Vec<usize> {
    map.entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.get("targetname").is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .map(|(i, _)| i)
        .collect()
}

fn sim(map: MapData) -> Sim {
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.ticks(2);
    sim
}

fn input(sim: &mut Sim, target: &str, input: &str, value: Value) {
    let mut logic = sim.app.world_mut().resource_mut::<Logic>();
    logic.world.queue_input(target, input, value, 0.0, None);
}

/// The prop node of map entity `index`.
fn prop_node(sim: &mut Sim, index: usize) -> Entity {
    let world = sim.app.world_mut();
    world
        .query::<(Entity, &PropEntity)>()
        .iter(world)
        .find(|(_, p)| p.0 == index)
        .map(|(e, _)| e)
        .unwrap_or_else(|| panic!("no prop node for entity {index}"))
}

#[test]
fn sprites_load_with_their_entity_and_follow_the_logic() {
    let Some(map) = load("cs_office") else { return };
    let glows = indices(&map, "projectorglow");
    assert_eq!(glows.len(), 14, "cs_office's projector glows");
    // All loaded (named, "Start on"), each knowing its entity.
    for i in &glows {
        let s = map.sprites.iter().find(|s| s.entity == Some(*i));
        assert!(s.is_some_and(|s| s.start_on), "sprite of entity {i}");
    }
    let Some(prodigy) = load("de_prodigy") else { return };
    // A named sprite without "Start on" loads too, hidden at start.
    let starts_off = prodigy
        .sprites
        .iter()
        .filter(|s| s.entity.is_some() && !s.start_on)
        .count();
    eprintln!(
        "de_prodigy: {} sprites, {starts_off} start hidden",
        prodigy.sprites.len()
    );

    let mut sim = sim(map);
    let parts = |sim: &Sim| {
        let logic = sim.app.world().resource::<Logic>();
        logic.world.part_states()
    };
    assert!(glows.iter().all(|i| parts(&sim).iter().any(|p| p.0 == *i && p.2)));
    // The projector's hum stops and its glow goes when shot (here: the
    // Kill its OnHealthChanged sends).
    input(&mut sim, "projectorglow", "Kill", Value::Void);
    sim.ticks(1);
    assert!(
        glows.iter().all(|i| !parts(&sim).iter().any(|p| p.0 == *i)),
        "killed glows are gone"
    );
    sim.app.world_mut().resource_mut::<RoundRestarts>().0 += 1;
    sim.ticks(1);
    assert!(
        glows.iter().all(|i| parts(&sim).iter().any(|p| p.0 == *i && p.2)),
        "back next round"
    );
}

#[test]
fn de_nuke_screens_change_skin_and_cars_play() {
    let Some(map) = load("de_nuke") else { return };
    let screens = indices(&map, "computer06");
    assert!(!screens.is_empty());
    let screen = screens[0];
    let prop = map
        .props
        .iter()
        .find(|p| p.entity == Some(screen))
        .expect("screen prop");
    let skins = map.models[prop.model].skins.len();
    assert!(skins >= 3, "the console has its screen skins ({skins})");
    let a = &map.models[prop.model].skins[0];
    let c = &map.models[prop.model].skins[2];
    assert!(
        a.iter().zip(c).any(|(a, c)| a.material != c.material),
        "skin 2 changes a material"
    );
    // The cars are animated: skeleton and sequences.
    let car = indices(&map, "car01")[0];
    let car_prop = map.props.iter().find(|p| p.entity == Some(car)).expect("car prop");
    let rig = map.models[car_prop.model].rig.as_ref().expect("the car has a rig");
    assert!(rig.animations.sequence("run").is_some() && rig.animations.sequence("hide").is_some());

    let mut sim = sim(map);
    let node = prop_node(&mut sim, screen);
    assert_eq!(sim.app.world().get::<PropLook>(node).map(|l| l.skin), Some(0));
    input(&mut sim, "computer06", "Skin", Value::Int(2));
    sim.ticks(1);
    assert_eq!(sim.app.world().get::<PropLook>(node).map(|l| l.skin), Some(2));
    let car_node = prop_node(&mut sim, car);
    let seq = sim.app.world().get::<PropSequence>(car_node).cloned().unwrap();
    assert_eq!((seq.play, seq.default.as_str()), (None, "hide"));
    input(&mut sim, "car01", "SetAnimation", Value::Str("run".into()));
    sim.ticks(1);
    let seq = sim.app.world().get::<PropSequence>(car_node).cloned().unwrap();
    assert_eq!(seq.play, Some(("run".to_string(), 1)));
}

#[test]
fn a_removed_prop_stops_blocking() {
    let Some(map) = load("de_nuke") else { return };
    let mut sim = sim(map);
    // A solid prop entity carries its own brushes (not the map's).
    let world = sim.app.world_mut();
    let (node, index, brushes) = world
        .query::<(Entity, &PropEntity, &MovingSolid)>()
        .iter(world)
        .find(|(_, _, m)| m.solid && !m.brushes.is_empty())
        .map(|(e, p, m)| (e, p.0, m.brushes.clone()))
        .expect("a solid prop entity");
    let in_map = world
        .resource::<MapBrushes>()
        .0
        .iter()
        .any(|b| b.min == brushes[0].min && b.max == brushes[0].max);
    assert!(!in_map, "its brushes are its own");
    let id = {
        let logic = world.resource::<Logic>();
        logic
            .world
            .ids()
            .into_iter()
            .find(|id| logic.world.get(*id).unwrap().map_index == Some(index))
            .unwrap()
    };
    world.resource_mut::<Logic>().world.kill(id);
    sim.ticks(1);
    let solid = sim.app.world().get::<MovingSolid>(node).unwrap().solid;
    assert!(!solid, "removed: movement passes it");
    let shot = sim.app.world().get::<ColliderDisabled>(node).is_some()
        || sim
            .app
            .world()
            .get::<Children>(node)
            .is_some_and(|c| c.iter().any(|c| sim.app.world().get::<ColliderDisabled>(c).is_some()));
    assert!(shot, "and shots pass it");
    sim.app.world_mut().resource_mut::<RoundRestarts>().0 += 1;
    sim.ticks(1);
    assert!(
        sim.app.world().get::<MovingSolid>(node).unwrap().solid,
        "back next round"
    );
}

#[test]
fn de_prodigy_lights_switch_their_lightmap_style() {
    let Some(map) = load("de_prodigy") else { return };
    let l = map.lightmap.as_ref().expect("lightmap");
    let styles: Vec<(u8, bool, usize)> = l.styles.iter().map(|s| (s.style, s.on, s.texels.len())).collect();
    eprintln!("de_prodigy switchable styles: {styles:?}");
    let s32 = l
        .styles
        .iter()
        .find(|s| s.style == 32)
        .expect("style 32 (bomb site B's lights)");
    assert!(s32.on && !s32.texels.is_empty());
    // As at map start: unchanged; style 32 off: darker where it lit.
    let (same, _) = l.relit(&|s| s.start());
    assert_eq!(same, l.rgb);
    let (off, _) = l.relit(&|s| if s.style == 32 { 0.0 } else { s.start() });
    let t = s32
        .rgb
        .iter()
        .position(|v| v[0] + v[1] + v[2] > 0.05)
        .expect("style 32 lights something");
    let at = s32.texels[t] as usize;
    let sum = |v: [f32; 3]| v[0] + v[1] + v[2];
    assert!(sum(off[at]) < sum(l.rgb[at]) - 0.04, "{:?} -> {:?}", l.rgb[at], off[at]);

    let mut sim = sim(map);
    let lit = |sim: &Sim, s: u8| {
        sim.app
            .world()
            .get_resource::<LightStyles>()
            .and_then(|l| l.styles.iter().find(|x| x.0 == s).map(|x| x.1))
    };
    assert_eq!(lit(&sim, 32), Some(true));
    input(&mut sim, "bombeffectsblights", "TurnOff", Value::Void);
    sim.ticks(1);
    assert_eq!(lit(&sim, 32), Some(false));
}

#[test]
fn de_aztec_soundscapes_follow_the_touch_code() {
    let Some(map) = load("de_aztec") else { return };
    let zones = &map.sounds.soundscape_zones;
    assert!(!zones.is_empty() && zones.iter().all(|z| z.entity.is_some()));
    let index = zones[0].entity.unwrap();
    let e = map.entities[index].clone();
    let pts: Vec<Vec3> = e.hulls.iter().flat_map(|h| h.points.clone()).collect();
    let (lo, hi) = pts
        .iter()
        .fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(*p), b.max(*p)));
    let centre = e.origin() + (lo + hi) / 2.0;
    let mut sim = sim(map);
    let p = sim.spawn_character(to_engine(centre), noclip::ID);
    sim.app.world_mut().entity_mut(p).insert(LocalPlayer);
    sim.ticks(2);
    let touches = sim.app.world().resource::<SoundscapeTouches>().clone();
    assert!(
        touches.0.as_ref().is_some_and(|t| t.contains(&index)),
        "inside trigger {index}: {touches:?}"
    );
}

/// Detail props (the BSP's detail lump): cs_militia's grass, 3025 "cross"
/// sprites (two blades each) and 8819 "tri" ones (three), fading out over
/// its env_detail_controller's 800-1024 units; de_inferno's detail bushes
/// are models, drawn as non-solid props with its controller's fade.
#[test]
fn detail_props_load() {
    let Some(map) = load("cs_militia") else { return };
    let d = map.detail_props.as_ref().expect("cs_militia detail sprites");
    assert_eq!(d.quads.len(), 3025 * 2 + 8819 * 3);
    assert_eq!(d.fade, Some((800.0 * 0.0254, 1024.0 * 0.0254)));
    assert!(d.quads.iter().all(|q| q.billboard.is_none()), "shapes are fixed blades");
    assert!(d.quads.iter().any(|q| q.sway > 0.0));
    // Lit: the sprites take their baked light, not black.
    assert!(d.quads.iter().filter(|q| q.light.iter().any(|c| *c > 0.05)).count() * 2 > d.quads.len());
    assert!(!mashup::map::detail::cell_meshes(d).is_empty());
    let Some(map) = load("de_inferno") else { return };
    assert!(map.detail_props.is_none(), "models only");
    let fade = Some((1024.0 * 0.0254, 1536.0 * 0.0254));
    let bushes = map
        .props
        .iter()
        .filter(|p| p.fade == fade && p.solid == mashup::map::PropSolid::None && p.entity.is_none())
        .count();
    assert_eq!(bushes, 53, "its 53 detail bushes");
}
