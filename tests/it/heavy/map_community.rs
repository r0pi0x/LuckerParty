//! Community CS:S maps from the user's content cache (never in the repo),
//! headless: the problems the map sweep found (`mapsweep`,
//! docs/plans/active/community-maps.md) stay fixed. Each test skips when
//! its map isn't in the cache or there is no install.

use bevy::prelude::*;
use mashup::{
    core::RoundRestarts,
    games::{
        self, cs_source,
        cs_source::{movement::SourceMovementPlugin, weapons::CsWeaponsPlugin},
    },
    harness::Sim,
    map::{MapData, MapPlugin},
    mount::config::{LocalConfig, content_dir},
    movement::placeholder,
    weapon::{
        Inventory, Weapon,
        drop::Loose,
        equip::{MapWeapon, SpawnEquipment},
    },
};

/// The map, when the install and the cached map are there.
fn load(name: &str) -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()
        .and_then(|c| c.game_path(cs_source::GAME))
        .is_some_and(|p| p.join("cstrike").is_dir());
    let cached = content_dir(cs_source::GAME).is_some_and(|d| d.join("maps").join(format!("{name}.bsp")).is_file());
    if !installed || !cached {
        eprintln!("skipping: {name} not in the content cache (or no install)");
        return None;
    }
    Some(games::load_map(&format!("cs_source:{name}")).unwrap_or_else(|e| panic!("{name}: {e}")))
}

/// Maps that failed to load: Latin-1 text in the entity lump
/// (gg_bk_warehouse_v1, mg_wipeout2), models whose animations sit in
/// `.ani` blocks (mg_kommando: vmdl panicked), a model whose `.vtx`
/// indexes past its `.vvd` (surf_surreal: vmdl panicked).
#[test]
fn maps_that_failed_load() {
    for name in ["gg_bk_warehouse_v1", "mg_wipeout2", "mg_kommando", "surf_surreal"] {
        let Some(map) = load(name) else { continue };
        assert!(!map.meshes.is_empty(), "{name}: no world");
        assert!(!map.spawns.is_empty(), "{name}: no spawns");
    }
}

/// MP3 sounds packed in the map play (mg_lego_multigames_v2's music).
#[test]
fn packed_mp3_sounds_decode() {
    let Some(map) = load("mg_lego_multigames_v2") else { return };
    let (entries, raw) = cs_source::sound::ambient_messages(&map.entities);
    let mp3: Vec<String> = entries.into_iter().chain(raw).filter(|s| s.ends_with(".mp3")).collect();
    assert!(!mp3.is_empty(), "the map's music");
    for s in &mp3 {
        let entry = map.sounds.entry(s).unwrap_or_else(|| panic!("{s}: no entry"));
        let clip = &map.sounds.clips[entry.waves[0]];
        assert!(clip.rate > 0 && clip.samples.len() > 1000, "{s}: decoded");
    }
}

/// A renamed map (bhop_backport_css, packed as kz_bhop_backport_css)
/// finds its baked cubemaps under the original name.
#[test]
fn renamed_map_finds_its_cubemaps() {
    let Some(map) = load("bhop_backport_css") else { return };
    let missing: Vec<_> = map.warnings.iter().filter(|w| w.contains("/c") && w.contains("not found")).collect();
    assert!(missing.is_empty(), "{missing:#?}");
    assert!(!map.cubemap_samples.is_empty());
}

/// Textures in uncompressed formats the vtf crate doesn't decode.
#[test]
fn uncommon_texture_formats_decode() {
    for name in ["gg_simpsons_bam", "mg_n64_goldeneye_v2", "mg_creative_multigames_v8_ns"] {
        let Some(map) = load(name) else { continue };
        let bad: Vec<_> = map.warnings.iter().filter(|w| w.contains("images is not supported")).collect();
        assert!(bad.is_empty(), "{name}: {bad:#?}");
    }
}

/// mg_item_battle_v4b: its game_player_equips (no "Use Only") give a P228
/// instead of the starting weapons, and its placed weapons lie where the
/// map puts them, put back at a round restart.
#[test]
fn spawn_equipment_and_placed_weapons() {
    let Some(map) = load("mg_item_battle_v4b") else { return };
    let placed = map
        .entities
        .iter()
        .filter(|e| e.classname().starts_with("weapon_"))
        .count();
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.ticks(2);
    let equipment = sim.app.world().resource::<SpawnEquipment>().0.clone();
    assert_eq!(equipment, Some(vec![("weapon_p228".to_string(), 1), ("weapon_p228".to_string(), 1)]));

    let p = sim.spawn_character(Vec3::new(0.0, 50.0, 0.0), placeholder::ID);
    sim.ticks(2);
    let held: Vec<&str> = {
        let world = sim.app.world();
        let inv = world.get::<Inventory>(p).expect("an inventory");
        inv.weapons.iter().filter_map(|w| world.get::<Weapon>(*w)).map(|w| w.id).collect()
    };
    assert_eq!(held, vec!["cs_source:weapon_p228"], "one P228 only");

    let lying = |sim: &mut Sim| {
        let world = sim.app.world_mut();
        world.query::<(&Loose, &MapWeapon)>().iter(world).count()
    };
    assert_eq!(lying(&mut sim), placed, "every placed weapon lies there");
    sim.app.world_mut().resource_mut::<RoundRestarts>().0 += 1;
    sim.ticks(2);
    assert_eq!(lying(&mut sim), placed, "put back, not doubled");
}

/// The map, when the install and the cached map are both there.
fn cached(name: &str) -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()
        .and_then(|c| c.game_path(cs_source::GAME))
        .is_some_and(|p| p.join("cstrike").is_dir());
    let present = content_dir(cs_source::GAME).is_some_and(|d| d.join("maps").join(format!("{name}.bsp")).is_file());
    if !installed || !present {
        eprintln!("skipping: {name} not in the content cache");
        return None;
    }
    Some(games::load_map(&format!("cs_source:{name}")).expect(name))
}

/// surf_boreas is repacked with LZMA-compressed lumps and its version-10
/// static prop lump sets the NO_DRAW bit on 670 of its 1587 props, its
/// surf ramps among them (`models/project_tendies/ramps/*`, 100 m long
/// models). The lighting must decode (not compressed bytes read as
/// luxels) and every static prop must be placed.
#[test]
fn surf_boreas_lighting_and_ramps() {
    let Some(map) = cached("surf_boreas") else { return };
    // Lightmaps: compressed bytes read as luxels gave random exponents.
    let lm = map.lightmap.as_ref().expect("lightmap");
    let wild = lm.rgb.iter().filter(|c| c.iter().any(|v| !v.is_finite() || *v > 16.0)).count();
    assert!(
        wild * 1000 < lm.rgb.len(),
        "{wild} of {} luxels out of range: lighting read from compressed bytes?",
        lm.rgb.len()
    );
    // Cubemap names come from the (compressed) cubemap lump.
    let bad: Vec<_> = map.warnings.iter().filter(|w| w.contains("materials/maps/surf_boreas/c")).collect();
    assert!(bad.is_empty(), "{bad:#?}");
    // Static props: all of them, the ramp models included.
    let statics = map.props.iter().filter(|p| p.entity.is_none()).count();
    assert!(statics >= 1587, "{statics} static props placed");
    let ramps = map
        .props
        .iter()
        .filter(|p| {
            let (lo, hi) = map.models[p.model].bounds;
            (hi - lo).max_element() > 90.0
        })
        .count();
    assert!(ramps >= 19, "{ramps} props over 90 m (the ramps)");
}
