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

    // A snow cave whose displacement folds back over itself: from inside
    // it (the "transition" soundscape, Source 10446 8619 9860), the floor
    // ahead must face the eye. Re-winding each triangle to the base face's
    // normal turned the folded ones inside out (culled: holes to the sky).
    let engine = |x: f32, y: f32, z: f32| Vec3::new(x, z, -y) * 0.0254;
    let eye = engine(10446.0, 8619.0, 9860.0);
    for dir in [Vec3::new(-0.526, -0.728, -0.44), Vec3::new(-0.876, 0.163, -0.454)] {
        let dir = Vec3::new(dir.x, dir.z, -dir.y).normalize();
        let hit = first_hit(&map, eye, dir).expect("the cave floor is drawn there");
        assert!(hit.1.dot(dir) < 0.0, "the nearest drawn triangle faces away from the eye: {hit:?}");
        assert!((7.0..13.0).contains(&hit.0), "the floor 8-12 m away, not further: {hit:?}");
    }
}

/// The nearest world triangle on a ray (distance, its front normal by
/// winding: counter-clockwise faces the viewer), skybox meshes left out.
fn first_hit(map: &MapData, o: Vec3, d: Vec3) -> Option<(f32, Vec3)> {
    let mut best: Option<(f32, Vec3)> = None;
    for m in map.meshes.iter().filter(|m| !m.skybox) {
        for t in m.indices.chunks_exact(3) {
            let [a, b, c] = [0, 1, 2].map(|k| Vec3::from_array(m.positions[t[k] as usize]));
            let (e1, e2) = (b - a, c - a);
            let h = d.cross(e2);
            let det = e1.dot(h);
            if det.abs() < 1e-9 {
                continue;
            }
            let s = o - a;
            let u = s.dot(h) / det;
            let q = s.cross(e1);
            let v = d.dot(q) / det;
            let t = e2.dot(q) / det;
            if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0 && best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, e1.cross(e2).normalize()));
            }
        }
    }
    best
}

/// Static props compiled with per-vertex lighting (`sp_<n>.vhv` in the
/// pakfile) take it instead of the light probe, and it agrees with the
/// probe in colour (same lighting, same units, same channel order).
#[test]
fn baked_prop_vertex_light() {
    for name in ["mg_lt_galaxy_v5", "surf_nebula", "kz_ancient_ruins", "surf_demise"] {
        let Some(map) = load(name) else { continue };
        let statics: Vec<_> = map.props.iter().filter(|p| p.entity.is_none()).collect();
        let baked: Vec<_> = statics.iter().filter(|p| p.vertex_light.is_some()).collect();
        // Per prop: the mean baked light over the mean probe light, per
        // channel; the median of each.
        let mut ratios: [Vec<f32>; 3] = Default::default();
        for p in &baked {
            let v = p.vertex_light.as_ref().unwrap();
            let (mut sum_v, mut sum_p) = (Vec3::ZERO, Vec3::ZERO);
            for m in &map.models[p.model].meshes {
                for (i, n) in m.normals.iter().enumerate() {
                    sum_v += Vec3::from(v[m.source_vertices[i] as usize]);
                    sum_p += p.lighting.as_ref().unwrap().eval(p.rotation * Vec3::from(*n));
                }
            }
            if sum_p.min_element() > 1e-3 {
                for c in 0..3 {
                    ratios[c].push(sum_v[c] / sum_p[c]);
                }
            }
        }
        let median = |v: &mut Vec<f32>| {
            v.sort_by(f32::total_cmp);
            v.get(v.len() / 2).copied().unwrap_or(0.0)
        };
        let m = ratios.each_mut().map(median);
        let unfit: Vec<_> = map.warnings.iter().filter(|w| w.contains("per-vertex")).collect();
        eprintln!(
            "{name}: {} of {} static props baked; median baked/probe light per channel {m:?}; {unfit:?}",
            baked.len(),
            statics.len(),
        );
        assert!(baked.len() * 10 >= statics.len() * 9, "{name}: most static props baked");
        assert!(unfit.is_empty(), "{name}: {unfit:?}");
        // Same hue as the probe (the files' B, G, R order), somewhat
        // darker (self-shadowing; docs/backlog.md's open question).
        let (lo, hi) = (m.iter().copied().fold(f32::MAX, f32::min), m.iter().copied().fold(0.0, f32::max));
        assert!(hi < 1.3 * lo, "{name}: channels {m:?}");
        assert!((0.3..1.5).contains(&lo), "{name}: brightness {m:?}");
    }
}

/// Animated light styles (1-31) keep their faces' share apart with their
/// pattern: bhop_myztek's flickering (style 1) and second flicker (6)
/// lights, mg_jacks_multigames_v1's candle (3) and fluorescent flicker
/// (10) next to its switchable styles.
#[test]
fn animated_light_styles_keep_their_pattern() {
    for (name, want) in [("bhop_myztek", vec![1u8, 6]), ("mg_jacks_multigames_v1", vec![1, 3, 6, 10])] {
        let Some(map) = load(name) else { continue };
        let l = map.lightmap.as_ref().expect("lightmap");
        let animated: Vec<u8> = l.styles.iter().filter(|s| !s.pattern.is_empty()).map(|s| s.style).collect();
        for style in &want {
            let s = l.styles.iter().find(|s| s.style == *style).unwrap_or_else(|| panic!("{name}: style {style}"));
            assert_eq!(s.pattern, cs_source::lightmap::style_pattern(*style, None), "{name}: style {style}");
            assert!(!s.rects.is_empty() && s.rgb.iter().any(|c| c[0] > 0.01), "{name}: style {style} lights something");
            let texels: u32 = s.rects.iter().map(|r| r[2] * r[3]).sum();
            assert_eq!(texels as usize, s.texels.len());
        }
        assert!(animated.iter().all(|s| *s < 32), "{animated:?}");
        if name == "mg_jacks_multigames_v1" {
            assert!(l.styles.iter().any(|s| s.style >= 32 && s.pattern.is_empty()), "switchable styles too");
        }
    }
}

/// Materials the VMT parser refused (the map sweep: 32 over 10 maps) read
/// the game's way: `$detailscale "[9 9 9]"`, WorldVertexTransition without
/// `$basetexture2`, an unreadable `$basetexturetransform`, a missing
/// closing brace; unknown shaders as their stand-ins (WindowImposter,
/// ShatteredGlass, `Refract_DX90`).
#[test]
fn lenient_materials() {
    let refused = [
        "Vec2OrSingle",
        "$basetexture2",
        "$basetexturetransform",
        "No valid token",
        "\"windowimposter\"",
        "\"shatteredglass\"",
        "\"lightmappedreflective\"",
        "\"refract_dx90\"",
        "Vec3OrSingle",
        "duplicate field",
        "missing field `$basetexture`",
    ];
    for name in [
        "kz_11342",
        "kz_hikari_od_nh_v2",
        "mg_kommando",
        "mg_escape_prison_beta",
        "surf_halloween_tf2",
        "surf_jive",
        "surf_threnody",
    ] {
        let Some(map) = load(name) else { continue };
        let bad: Vec<_> = map
            .warnings
            .iter()
            .filter(|w| refused.iter().any(|r| w.contains(r)))
            .collect();
        assert!(bad.is_empty(), "{name}: {bad:#?}");
        if name == "surf_threnody" {
            // Its fake skies show their cubemap.
            assert!(
                map.meshes
                    .iter()
                    .any(|m| m.material.contains("fakeskies") && m.envmap.is_some_and(|e| e.imposter)),
                "a WindowImposter surface"
            );
        }
    }
}

/// mg_swag_multigames_v1 places about 250 weapons. Every resting one swept
/// itself (swept CCD) against all it touched each tick, the world's
/// colliders included: 19 ms a tick, so frames took 340 ms (ticks piled
/// up). Resting items sweep nothing now.
#[test]
fn resting_placed_weapons_cost_no_sweeps() {
    let Some(map) = load("mg_swag_multigames_v1") else { return };
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    // Let them land and settle.
    sim.ticks(200);
    let placed = {
        let world = sim.app.world_mut();
        world.query::<(&Loose, &MapWeapon)>().iter(world).count()
    };
    assert!(placed > 200, "{placed} placed weapons");
    let mut worst = std::time::Duration::ZERO;
    type Diagnostics = avian3d::dynamics::solver::SolverDiagnostics;
    for _ in 0..20 {
        *sim.app.world_mut().resource_mut::<Diagnostics>() = Diagnostics::default();
        sim.ticks(1);
        worst = worst.max(sim.app.world().resource::<Diagnostics>().swept_ccd);
    }
    let moving = {
        let world = sim.app.world_mut();
        world
            .query_filtered::<&avian3d::prelude::LinearVelocity, With<MapWeapon>>()
            .iter(world)
            .filter(|v| v.length() > 0.5)
            .count()
    };
    eprintln!("swept CCD at most {worst:?} a tick; {moving} of {placed} weapons moving");
    // 55 ms before, 0.1-3.5 ms after (machine load).
    assert!(worst.as_secs_f32() < 0.02, "swept CCD took {worst:?} in a tick");
}

/// surf_demise: its ramps are translucent marble over an envmap-only
/// material (no `$basetexture`) reflecting a tinted HDR sky cubemap: black
/// albedo (specs/cs_source/shaders.md 2), not the grey stand-in (which
/// read as a magenta floor at the spawn). The cubemap is a half-float VTF
/// in an LZMA-compressed pak entry the zip reader's decoder refused.
#[test]
fn envmap_only_ramps_reflect_their_sky() {
    let Some(map) = load("surf_demise") else { return };
    let bad: Vec<_> = map.warnings.iter().filter(|w| w.contains("sky_demise_05")).collect();
    assert!(bad.is_empty(), "{bad:#?}");
    let sky: Vec<_> = map
        .models
        .iter()
        .flat_map(|m| &m.meshes)
        .filter(|m| m.material.contains("sky_demise_05"))
        .collect();
    assert!(!sky.is_empty(), "the ramps' fake-sky meshes");
    for m in sky {
        let texture = &map.textures[m.texture.expect("a black base texture")];
        assert_eq!(&texture.rgba8[..3], &[0, 0, 0], "{}: black albedo", m.material);
        assert!(m.envmap.is_some_and(|e| e.cubemap.is_some()), "{}: its sky cubemap", m.material);
    }
}

/// Material effects community maps use (specs/cs_source/shaders.md 2, 4,
/// 6 and open questions 16 and 18): detail blend modes past 0 and 1,
/// `$selfillum`, `$basetexturetransform` and its TextureScroll proxy, sky
/// faces' half-height transform.
#[test]
fn material_effects() {
    use mashup::map::{DetailMode, MapMesh};
    fn meshes(map: &MapData) -> impl Iterator<Item = &MapMesh> {
        map.meshes.iter().chain(map.models.iter().flat_map(|m| &m.meshes))
    }
    if let Some(map) = load("kz_ancient_ruins") {
        // A temple prop's moss: detail mode 2 (model shader: decoded).
        let moss = meshes(&map)
            .find(|m| m.material.contains("doorways_moss"))
            .expect("the doorway moss");
        let d = moss.detail.expect("its detail");
        assert_eq!(d.mode, DetailMode::Source(2));
        assert!(map.textures[d.texture].srgb, "model detail other than mod2x is sRGB-decoded");
        // Self-lit props (base alpha masks).
        assert!(meshes(&map).any(|m| m.selfillum.is_some()), "a self-illuminated surface");
        // A scrolling texture (TextureScroll on $basetexturetransform).
        assert!(
            meshes(&map).any(|m| m.base_transform.scroll != [0.0, 0.0]),
            "a scrolling base texture"
        );
    }
    if let Some(map) = load("surf_demise") {
        if let Some(bone) = meshes(&map).find(|m| m.material.contains("bonecolor")) {
            assert_eq!(bone.detail.map(|d| d.mode), Some(DetailMode::Source(8)));
        }
    }
    if let Some(map) = load("bhop_flatzone") {
        // Half-height sky faces: "center 0 0 scale 1 2" on the sides.
        let sky = map.sky.as_ref().expect("a sky");
        assert!(sky.transforms.iter().any(|t| !t.is_identity()), "sky face transforms");
    }
}

/// mg_item_battle_v4b's items: placed knives with entities parented to
/// them (a car, a jetpack...). Walking onto one picks it up, its map
/// entity fires OnPlayerPickup with the player as activator, and what is
/// parented to it follows the player: props ride the knife's anchor node
/// (at the middle of the player's box, 31 units above the feet standing,
/// turned to its yaw), keeping their placement. CS:S's captures of the car
/// knife (first and third person) put the car there: on the floor, roof
/// at the carrier's shoulders; at the feet it sank to the carrier's knees.
#[test]
fn item_children_follow_the_player() {
    use mashup::map::{
        PropEntity,
        entities::{MapAnchor, anchor_entities, parent_name},
    };
    let Some(map) = load("mg_item_battle_v4b") else { return };
    let anchors = anchor_entities(&map.entities);
    let items: Vec<usize> = (0..map.entities.len()).filter(|i| anchors[*i]).collect();
    assert!(!items.is_empty(), "placed weapons with entities parented to them");
    let item = items[0];
    let name = map.entities[item].get("targetname").unwrap().to_string();
    let children: Vec<usize> = map
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.get("parentname").is_some_and(|p| parent_name(p).eq_ignore_ascii_case(&name)))
        .map(|(i, _)| i)
        .collect();
    assert!(!children.is_empty());
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.ticks(2);
    sim.app.world_mut().resource_mut::<mashup::logic::Logic>().world.record = true;
    let lying = |sim: &mut Sim| {
        let world = sim.app.world_mut();
        world
            .query::<(&Loose, &MapWeapon, &Transform)>()
            .iter(world)
            .find(|(_, m, _)| m.0 == item)
            .map(|(l, _, t)| (l.weapon, t.translation))
    };
    let (weapon, at) = lying(&mut sim).expect("the item lies where the map put it");
    let node = |sim: &mut Sim, index: usize| {
        let world = sim.app.world_mut();
        world
            .query::<(&PropEntity, &GlobalTransform)>()
            .iter(world)
            .find(|(p, _)| p.0 == index)
            .map(|(_, g)| g.translation())
    };
    let anchor = |sim: &mut Sim| {
        let world = sim.app.world_mut();
        world
            .query::<(&MapAnchor, &GlobalTransform)>()
            .iter(world)
            .find(|(a, _)| a.0 == item)
            .map(|(_, g)| g.translation())
            .expect("the item's anchor node")
    };
    let props: Vec<(usize, f32)> = children
        .iter()
        .filter_map(|c| Some((*c, node(&mut sim, *c)?.distance(anchor(&mut sim)))))
        .collect();
    // Walk onto it: picked up.
    let p = sim.spawn_character(at + Vec3::Y * 0.1, mashup::games::cs_source::movement::ID);
    sim.ticks(10);
    let owner = sim.app.world().get::<Weapon>(weapon).and_then(|w| w.owner);
    assert_eq!(owner, Some(p), "picked up");
    {
        let logic = sim.app.world().resource::<mashup::logic::Logic>();
        let id = logic.world.by_map_index(item).expect("the item's entity");
        assert!(
            logic.world.fired.iter().any(|(_, e, o)| *e == id && o == "OnPlayerPickup"),
            "OnPlayerPickup"
        );
        for line in logic.world.log.iter().filter(|l| l.contains("unhandled")).take(20) {
            eprintln!("logic: {line}");
        }
    }
    // Carried away: the anchor follows the player, the props keep their
    // distance from it.
    let away = at + Vec3::new(8.0, 0.0, 3.0);
    sim.app.world_mut().get_mut::<Transform>(p).unwrap().translation = away;
    sim.seconds(1.0);
    let a = anchor(&mut sim);
    assert!(a.xz().distance(sim.position(p).xz()) < 0.05, "anchor {a} at the player {}", sim.position(p));
    // Standing on the floor (what rides along doesn't lift its carrier),
    // the anchor 31 units above the feet.
    let state = sim.state(p);
    assert!(state.on_ground, "the carrier stands");
    let feet = sim.position(p).y + state.hull_min.y;
    let above = (a.y - feet) / cs_source::bsp::METERS_PER_UNIT;
    assert!((above - 31.0).abs() < 0.1, "anchor {above} units above the feet");
    for (c, d) in props {
        let now = node(&mut sim, c).unwrap();
        assert!((now.distance(a) - d).abs() < 0.05, "prop {c}: {d} from the anchor, now {}", now.distance(a));
    }
}

/// mg_creative_multigames_v8_ns's minigame rooms are walled off from each
/// other by sky faces. From the playtest's spot (`setpos -6332.74 -3047.50
/// -9638.97`) other rooms' placed weapons and water showed through the
/// sky: they weren't culled by the map's visibility as the world is (and
/// as Source culls every entity). Now loose items, moving props and water
/// surfaces (in chunks) are drawn exactly when one of their clusters is
/// potentially visible.
#[test]
fn other_rooms_items_and_water_are_culled_on_creative_multigames() {
    let Some(map) = load("mg_creative_multigames_v8_ns") else { return };
    assert!(!map.sky_surfaces.is_empty() && map.sky_surfaces.len() % 3 == 0, "sky faces kept");
    let v = map.visibility.clone().expect("visibility");
    let eye = Vec3::new(-6332.74, -9638.97 + 64.0, 3047.50) * cs_source::bsp::METERS_PER_UNIT;
    let cluster = v.cluster_at(eye).expect("the spot is in a cluster");
    let mut sim = Sim::new((
        |app: &mut App| {
            app.init_asset::<Image>()
                .init_asset::<StandardMaterial>()
                .init_asset::<mashup::map::world_material::WorldMaterial>()
                .init_asset::<mashup::map::prop_material::PropMaterial>()
                .init_asset::<mashup::map::sprite_material::SpriteMaterial>()
                .init_asset::<mashup::map::water::WaterMaterial>()
                .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>();
        },
        MapPlugin::new(map),
        SourceMovementPlugin,
        CsWeaponsPlugin,
    ));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    let view = sim.app.world_mut().spawn((Camera3d::default(), Transform::from_translation(eye))).id();
    sim.ticks(1);
    // The map's own cameras (the water's reflection, the sky's) are
    // switched by systems that need rendering: off here.
    let world = sim.app.world_mut();
    let mut cams = world.query::<(Entity, &mut Camera)>();
    for (e, mut c) in cams.iter_mut(world) {
        c.is_active = e == view;
    }
    // Placed weapons settle (they fall onto what they rest on).
    sim.seconds(3.0);
    let world = sim.app.world_mut();
    assert_eq!(world.resource::<mashup::map::vis::VisStats>().cluster, Some(cluster));
    let check = |what: &str, parts: Vec<(Box<[u32]>, bool)>| {
        let hidden = parts.iter().filter(|(_, on)| !on).count();
        for (clusters, on) in &parts {
            assert!(!clusters.is_empty(), "{what}: untagged");
            assert_eq!(*on, v.sees_any(cluster, clusters), "{what}: clusters {clusters:?}");
        }
        eprintln!("{what}: {} drawn, {hidden} culled", parts.len() - hidden);
        assert!(hidden > 0, "{what}: other rooms' culled");
    };
    let mut q = world.query_filtered::<&mashup::map::vis::VisClusters, With<mashup::map::loose::LooseItem>>();
    let items = q.iter(world).map(|c| (c.clusters.clone(), c.potentially_visible)).collect();
    check("placed weapons", items);
    let mut q = world.query::<(&mashup::map::water::WaterSurface, Option<&mashup::map::vis::VisClusters>)>();
    let water = q
        .iter(world)
        .filter(|(s, _)| !s.skybox)
        .map(|(_, c)| c.map_or((Box::default(), true), |c| (c.clusters.clone(), c.potentially_visible)))
        .collect();
    check("water surfaces", water);
}

/// The 2D sky's bottom face (`dn`) joins the side faces: on community
/// skies whose bottom is a picture (stock CS:S bottoms are one colour),
/// the orientation `cs_source::sky::FACES` gives it has the smallest seam
/// of the eight (`map::sky_seam_error`; the top face's fitted one wins
/// the same way on stock skies: `map_stock::sky_top_and_bottom_join_the_sides`).
#[test]
fn sky_bottom_joins_the_sides() {
    let names = [
        "gg_lego_spacetower2",
        "kz_hikari_od_nh_v2",
        "kz_bhop_izanami",
        "surf_happyhands",
        "surf_stickybutt_alpha",
        "gg_dev_moment_v1",
        "gg_desert_paintball",
        "gg_mario_vs_wario",
        "gg_fusion_trx",
        "mg_kommando",
    ];
    let dn = cs_source::sky::FACES.iter().find(|f| f.0 == "dn").unwrap();
    let mut decided = 0;
    for name in names {
        let Some(map) = load(name) else { continue };
        let Some(sky) = map.sky.as_ref() else { continue };
        let errors: Vec<f32> = (0..8u8)
            .map(|turns| {
                let mut s = sky.clone();
                s.faces[dn.1].1 = turns;
                mashup::map::sky_seam_error(&s, &map.textures, dn.1)
            })
            .collect();
        eprintln!("{name}: {errors:?}");
        // One-colour bottoms join every way alike.
        let (lo, hi) = errors.iter().fold((f32::MAX, f32::MIN), |(a, b), e| (a.min(*e), b.max(*e)));
        if hi - lo < 2.0 {
            continue;
        }
        let best = (0..8).min_by(|a, b| errors[*a].total_cmp(&errors[*b])).unwrap();
        assert_eq!(best, dn.2 as usize, "{name}: {errors:?}");
        decided += 1;
    }
    eprintln!("{decided} skies decide the bottom face");
}

/// Visual entities the logic audit found undrawn (community-maps.md, "Map
/// logic audit, round 2"): gg_future's spinning laser trails
/// (env_spritetrail), gg_nukkon_hdr's cooling-tower smoke
/// (env_smokestack), the `.pcf` effects of info_particle_system from the
/// install (surf_hellenic's env_fire_large) and from the map's own pak
/// (mg_crazykart_v1_1's kart effects), and surf_demise's faint
/// func_illusionary glass (rendermode 1, renderamt 20) drawn blended.
#[test]
fn visual_entities_load() {
    use mashup::map::MapAlpha;
    if let Some(map) = load("gg_future") {
        assert_eq!(map.trails.len(), 4);
        assert!(map.trails.iter().all(|t| t.life == 10.0 && t.entity.is_some()));
    }
    if let Some(map) = load("gg_nukkon_hdr") {
        assert_eq!(map.smokestacks.len(), 4);
        assert!(
            map.smokestacks
                .iter()
                .all(|s| s.life().is_some_and(|l| (l - 10.0).abs() < 1e-3))
        );
    }
    if let Some(map) = load("surf_hellenic") {
        let ps = &map.particle_systems;
        assert_eq!(ps.placed.len(), 160);
        let fire = ps
            .find("env_fire_large")
            .expect("env_fire_large from the install's fire_01.pcf");
        assert!(!ps.defs[fire].children.is_empty(), "its flames and embers");
        assert!(ps.defs[fire].material.is_some());
        assert!(
            !map.warnings.iter().any(|w| w.contains("particle")),
            "{:?}",
            map.warnings
        );
    }
    if let Some(map) = load("mg_crazykart_v1_1") {
        let ps = &map.particle_systems;
        assert_eq!(ps.placed.len(), 40);
        assert!(ps.find("kart_boost").is_some(), "from the map's packed crazykart.pcf");
        assert!(
            !map.warnings.iter().any(|w| w.contains("particle")),
            "{:?}",
            map.warnings
        );
    }
    if let Some(map) = load("surf_demise") {
        let faint = map
            .meshes
            .iter()
            .filter(|m| matches!(m.render, Some((MapAlpha::Blend, a)) if (a - 20.0 / 255.0).abs() < 1e-4))
            .count();
        assert!(faint > 0, "the func_illusionary glass is drawn faint");
        assert!(
            map.meshes
                .iter()
                .filter(|m| m.render.is_some())
                .all(|m| m.alpha == MapAlpha::Blend)
        );
    }
}
