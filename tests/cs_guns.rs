//! Scenario tests for CS:S's guns beyond the AK-47 (M4A1, AWP, USP, Glock,
//! Deagle) on the greybox map at CS:S's tick (0.015 s): script values
//! (specs/cs_source/weapons.md, "Weapon data") and the measured rules
//! ("CS:S values (measured)": M3 recoil, M4/M8 timing, M5-M7 damage, M15
//! zoom, M16 fire modes).

use bevy::prelude::*;
use mashup::{
    console::Console,
    core::{Health, LocalPlayer, MaxSpeed, Team},
    games::cs_source::{
        TICK_INTERVAL,
        weapons::{AWP, CsWeaponsPlugin, DEAGLE, GLOCK, GUNS, Gun, Inaccuracy, M4A1, USP},
    },
    greybox::{self, GreyboxMapPlugin},
    harness::Sim,
    movement::placeholder,
    weapon::{AltModes, Armor, Hitscan, Inventory, Magazine, SpreadShape, ViewPunch, Weapon, Zoomed, give},
};

const UNIT: f32 = 0.0254;

fn sim() -> Sim {
    let mut sim = Sim::new((GreyboxMapPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim
}

fn gun(id: &str) -> &'static Gun {
    GUNS.iter().find(|g| g.id == id).unwrap()
}

fn active(sim: &Sim, p: Entity) -> Entity {
    sim.app
        .world()
        .get::<Inventory>(p)
        .unwrap()
        .active
        .expect("no active weapon")
}

fn active_id(sim: &Sim, p: Entity) -> &'static str {
    sim.app.world().get::<Weapon>(active(sim, p)).unwrap().id
}

fn clip(sim: &Sim, p: Entity) -> u32 {
    sim.app.world().get::<Magazine>(active(sim, p)).unwrap().clip
}

fn mode(sim: &Sim, p: Entity) -> u8 {
    sim.app.world().get::<AltModes>(active(sim, p)).unwrap().current
}

/// A shooter holding `id`, drawn (one tick to give, then the draw time).
fn shooter_with(sim: &mut Sim, id: &str) -> Entity {
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.ticks(1);
    give(sim.app.world_mut(), p, id).unwrap();
    sim.ticks(1);
    assert_eq!(active_id(sim, p), id);
    sim.seconds(gun(id).draw as f64 + 0.05);
    p
}

/// Look from `p`'s eye at `target`.
fn aim_at(sim: &mut Sim, p: Entity, target: Vec3) {
    let eye = sim.position(p) + sim.state(p).eye_offset;
    let d = target - eye;
    let mut i = sim.intent(p);
    i.yaw = (-d.x).atan2(-d.z);
    i.pitch = d.y.atan2(d.xz().length());
}

/// Press attack for one tick, then release for one.
fn tap(sim: &mut Sim, p: Entity) {
    sim.intent(p).fire = true;
    sim.ticks(1);
    sim.intent(p).fire = false;
    sim.ticks(1);
}

fn press2(sim: &mut Sim, p: Entity) {
    sim.intent(p).secondary = true;
    sim.ticks(1);
    sim.intent(p).secondary = false;
}

/// Ticks until the clip changes, with the buttons as they are.
fn ticks_until_shot(sim: &mut Sim, p: Entity, limit: u32) -> Option<u32> {
    let c = clip(sim, p);
    for t in 1..=limit {
        sim.ticks(1);
        if clip(sim, p) != c {
            return Some(t);
        }
    }
    None
}

/// Health points `p` lost from `start` (normalized).
fn lost(sim: &Sim, p: Entity, start: f32) -> i32 {
    ((start - sim.app.world().get::<Health>(p).unwrap().current) * 100.0).round() as i32
}

/// One shot from `id` at a target `meters` ahead, at its chest or head
/// (`head`), the target with 1000 hp (as the probe's) and optionally
/// armour with a helmet. Returns (health lost, expected): expected =
/// int(Damage x RangeModifier^(d/500) x group [x ArmorRatio x 0.5]) with d
/// the distance from the eye to the capsule's near side (M5-M7).
fn hit(id: &str, meters: f32, head: bool, armour: bool) -> (i32, i32) {
    let g = gun(id);
    let mut sim = sim();
    let shooter = shooter_with(&mut sim, id);
    let target = sim.spawn_character(greybox::SPAWNS[0] - Vec3::Z * meters, placeholder::ID);
    sim.app.world_mut().get_mut::<Health>(target).unwrap().current = 10.0;
    if armour {
        sim.app.world_mut().entity_mut(target).insert(Armor {
            amount: 1.0,
            helmet: true,
        });
    }
    sim.ticks(2);
    let height = if head { 1.8 * 0.92 - 0.9 } else { 1.8 * 0.72 - 0.9 };
    let point = sim.position(target) + Vec3::Y * height;
    aim_at(&mut sim, shooter, point);
    // Standing still; the AWP scoped (unscoped its cone is 0.08: misses
    // at 8 m), and settled to its scoped accuracy.
    if id == AWP {
        press2(&mut sim, shooter);
        sim.seconds(1.5);
    }
    tap(&mut sim, shooter);
    let eye = sim.position(shooter) + sim.state(shooter).eye_offset;
    let d = ((point - eye).length() - 0.4) / UNIT;
    let group = if head { 4.0 } else { 1.0 };
    let mut damage = g.damage * g.range_modifier.powf(d / 500.0) * group;
    if armour {
        damage *= g.armor_ratio * 0.5;
    }
    // Keep away from integer boundaries, where d's estimate matters.
    let frac = damage.fract();
    assert!(
        (0.05..0.95).contains(&frac),
        "{id} at {meters} m: {damage} too close to an integer"
    );
    (lost(&sim, target, 10.0), damage as i32)
}

// ---------------------------------------------------------------------------
// Starting weapons, buying, slots

#[test]
fn teams_start_with_their_pistol_and_the_rifle_drawn() {
    let mut sim = sim();
    let t = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.app.world_mut().entity_mut(t).insert(Team(1));
    let ct = sim.spawn_character(greybox::SPAWNS[1], placeholder::ID);
    sim.app.world_mut().entity_mut(ct).insert(Team(2));
    sim.ticks(1);
    let ids = |sim: &Sim, p: Entity| -> Vec<&'static str> {
        let inv = sim.app.world().get::<Inventory>(p).unwrap();
        inv.weapons
            .iter()
            .map(|w| sim.app.world().get::<Weapon>(*w).unwrap().id)
            .collect()
    };
    use mashup::games::cs_source::weapons::{AK47, KNIFE};
    assert_eq!(ids(&sim, t), [KNIFE, GLOCK, AK47]);
    assert_eq!(ids(&sim, ct), [KNIFE, USP, AK47]);
    // Slot keys: 2 (index 1) the pistol.
    sim.intent(t).select = Some(1);
    sim.ticks(1);
    assert_eq!(active_id(&sim, t), GLOCK);
}

#[test]
fn buy_gives_each_gun_by_its_cs_name() {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.app.world_mut().entity_mut(p).insert(LocalPlayer);
    sim.ticks(1);
    for (name, id, slot) in [
        ("m4a1", M4A1, 0),
        ("awp", AWP, 0),
        ("usp", USP, 1),
        ("glock", GLOCK, 1),
        ("deagle", DEAGLE, 1),
    ] {
        sim.app
            .world_mut()
            .resource_mut::<Console>()
            .submit(format!("buy {name}"));
        sim.ticks(2);
        assert_eq!(active_id(&sim, p), id, "buy {name}");
        let w = sim.app.world().get::<Weapon>(active(&sim, p)).unwrap();
        assert_eq!(w.slot, slot, "{name} slot");
    }
}

// ---------------------------------------------------------------------------
// Damage (M5-M7)

#[test]
fn chest_damage_falls_off_by_each_guns_range_modifier() {
    for (id, meters) in [(M4A1, 8.0), (AWP, 8.0), (USP, 8.0), (GLOCK, 5.0), (DEAGLE, 8.0)] {
        let (got, want) = hit(id, meters, false, false);
        assert_eq!(got, want, "{id} chest at {meters} m");
    }
}

#[test]
fn helmet_headshots_take_the_armour_ratio() {
    for (id, meters) in [(M4A1, 6.0), (AWP, 5.0), (USP, 5.0), (GLOCK, 5.0), (DEAGLE, 6.0)] {
        let (got, want) = hit(id, meters, true, true);
        assert_eq!(got, want, "{id} head with helmet at {meters} m");
    }
}

#[test]
fn awp_chest_hit_matches_the_measured_114() {
    // M13: the AWP alone at 300 units: 114 on the chest.
    let (got, _) = hit(AWP, 8.0, false, false);
    assert_eq!(got, 114);
}

// ---------------------------------------------------------------------------
// Fire timing, deploy, reload (M4, M8)

#[test]
fn deploy_times_are_the_draw_sequences() {
    // First shot ceil(draw / tick) ticks after the draw: M4A1 0.975 s -> 65
    // (measured), Glock 1.0667 s -> 72, the others 1.0 s -> 67.
    for (id, ticks) in [(M4A1, 65), (AWP, 67), (USP, 67), (GLOCK, 72), (DEAGLE, 67)] {
        let mut sim = sim();
        let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
        sim.ticks(1);
        give(sim.app.world_mut(), p, id).unwrap();
        sim.ticks(1); // drawn this tick
        sim.intent(p).fire = true;
        let first = ticks_until_shot(&mut sim, p, 200).expect("never fired");
        assert_eq!(first, ticks, "{id}");
    }
}

#[test]
fn m4a1_held_fires_every_six_ticks() {
    // M4: CycleTime 0.09 held: 6 ticks every time.
    let mut sim = sim();
    let p = shooter_with(&mut sim, M4A1);
    sim.intent(p).fire = true;
    ticks_until_shot(&mut sim, p, 5).unwrap();
    let gaps: Vec<u32> = (0..6).map(|_| ticks_until_shot(&mut sim, p, 20).unwrap()).collect();
    assert_eq!(gaps, [6; 6]);
}

#[test]
fn semi_automatics_fire_once_per_press_at_their_cycle() {
    // M4: held fires once; presses fire when the cycle has passed: USP and
    // Glock 10 ticks, Deagle 15 (16 when float rounding lands past), AWP 100.
    for (id, ticks) in [
        (USP, [10, 10]),
        (GLOCK, [10, 10]),
        (DEAGLE, [15, 16]),
        (AWP, [100, 100]),
    ] {
        let mut sim = sim();
        let p = shooter_with(&mut sim, id);
        sim.intent(p).fire = true;
        ticks_until_shot(&mut sim, p, 2).unwrap();
        assert_eq!(ticks_until_shot(&mut sim, p, 150), None, "{id} held fired again");
        // Re-press every tick: fires as soon as it may.
        let mut sim = self::sim();
        let p = shooter_with(&mut sim, id);
        let mut shots = Vec::new();
        let mut last = 0;
        for t in 0..300u32 {
            let c = clip(&sim, p);
            sim.intent(p).fire = t % 2 == 0;
            sim.ticks(1);
            if clip(&sim, p) != c {
                if !shots.is_empty() || last > 0 {
                    shots.push(t - last);
                }
                last = t;
            }
            if shots.len() == 2 {
                break;
            }
        }
        // Presses land on even ticks, so a cycle can round up one.
        for gap in &shots {
            assert!((ticks[0]..=ticks[1] + 1).contains(gap), "{id}: {shots:?}");
        }
    }
}

#[test]
fn reloads_take_the_reload_sequences() {
    // ceil(reload / tick): M4A1 204 (measured), AWP 245, USP 179, Glock 143,
    // Deagle 145; no rounds lost.
    for (id, ticks) in [(M4A1, 204), (AWP, 245), (USP, 179), (GLOCK, 143), (DEAGLE, 145)] {
        let g = gun(id);
        let mut sim = sim();
        let p = shooter_with(&mut sim, id);
        let w = active(&sim, p);
        sim.app.world_mut().get_mut::<Magazine>(w).unwrap().clip = 1;
        sim.intent(p).reload = true;
        sim.ticks(1);
        sim.intent(p).reload = false;
        sim.ticks(ticks - 1);
        assert_eq!(clip(&sim, p), 1, "{id} swapped too early");
        sim.ticks(1);
        let m = sim.app.world().get::<Magazine>(w).unwrap();
        assert_eq!((m.clip, m.reserve), (g.clip, g.ammo.max - (g.clip - 1)), "{id}");
    }
}

// ---------------------------------------------------------------------------
// Recoil and inaccuracy (M1-M3)

/// The view punch after one shot, standing or crouched, degrees.
fn first_kick(id: &str, crouched: bool) -> Option<Vec2> {
    let mut sim = sim();
    let p = shooter_with(&mut sim, id);
    if crouched {
        sim.intent(p).crouch = true;
        sim.seconds(1.0);
    }
    sim.intent(p).fire = true;
    sim.ticks(1);
    sim.intent(p).fire = false;
    let p = sim.app.world().get::<ViewPunch>(p)?.0;
    Some(Vec2::new(p.x.to_degrees(), p.y.to_degrees()))
}

#[test]
fn first_shot_kicks_from_the_measured_sets() {
    // M3: M4A1 standing 0.65 up / 0.35 side, crouched 0.6 / 0.3; USP,
    // Deagle, AWP 2.0 straight up; the Glock not at all.
    let close = |k: Vec2, up: f32, side: f32| (k.x - up).abs() < 1e-3 && (k.y.abs() - side).abs() < 1e-3;
    let k = first_kick(M4A1, false).unwrap();
    assert!(close(k, 0.65, 0.35), "M4A1 {k}");
    let k = first_kick(M4A1, true).unwrap();
    assert!(close(k, 0.6, 0.3), "M4A1 crouched {k}");
    for id in [USP, DEAGLE, AWP] {
        let k = first_kick(id, false).unwrap();
        assert!(close(k, 2.0, 0.0), "{id} {k}");
    }
    assert!(first_kick(GLOCK, false).is_none_or(|k| k == Vec2::ZERO));
}

#[test]
fn usp_shots_add_its_fire_inaccuracy() {
    // M1: USP 0.008 -> 0.04295 on the shot (+InaccuracyFire 0.03495).
    let mut sim = sim();
    let p = shooter_with(&mut sim, USP);
    sim.seconds(2.0);
    let w = active(&sim, p);
    let before = sim.app.world().get::<Inaccuracy>(w).unwrap().value;
    assert!((before - 0.008).abs() < 1e-5, "{before}");
    sim.intent(p).fire = true;
    sim.ticks(1);
    let after = sim.app.world().get::<Inaccuracy>(w).unwrap().value;
    assert!((after - 0.04295).abs() < 1e-5, "{after}");
}

// ---------------------------------------------------------------------------
// AWP zoom (M15)

fn zoomed(sim: &Sim, p: Entity) -> Option<Zoomed> {
    sim.app.world().get::<Zoomed>(p).copied()
}

fn speed(sim: &Sim, p: Entity) -> f32 {
    sim.app.world().get::<MaxSpeed>(p).unwrap().0 / UNIT
}

#[test]
fn awp_zooms_40_then_10_then_off_and_slows() {
    let mut sim = sim();
    let p = shooter_with(&mut sim, AWP);
    assert!(zoomed(&sim, p).is_none());
    assert!((speed(&sim, p) - 210.0).abs() < 1e-3);
    press2(&mut sim, p);
    sim.ticks(1);
    assert_eq!(zoomed(&sim, p).map(|z| z.fov), Some(40.0));
    assert!(zoomed(&sim, p).unwrap().scope);
    assert!((speed(&sim, p) - 150.0).abs() < 1e-3, "zoomed speed");
    // Within 0.3 s another press does nothing.
    sim.ticks(5);
    press2(&mut sim, p);
    sim.ticks(1);
    assert_eq!(zoomed(&sim, p).map(|z| z.fov), Some(40.0));
    sim.seconds(0.3);
    press2(&mut sim, p);
    sim.ticks(1);
    assert_eq!(zoomed(&sim, p).map(|z| z.fov), Some(10.0));
    sim.seconds(0.3);
    press2(&mut sim, p);
    sim.ticks(1);
    assert!(zoomed(&sim, p).is_none());
    assert!((speed(&sim, p) - 210.0).abs() < 1e-3);
}

#[test]
fn awp_scoped_is_accurate_and_a_shot_unzooms_until_it_can_fire_again() {
    let mut sim = sim();
    let p = shooter_with(&mut sim, AWP);
    let w = active(&sim, p);
    let cone = |sim: &Sim| match sim.app.world().get::<Hitscan>(w).unwrap().spread {
        SpreadShape::Disc { inaccuracy, .. } => inaccuracy,
        _ => unreachable!(),
    };
    sim.ticks(1);
    assert!((cone(&sim) - 0.0808).abs() < 1e-4, "unscoped {}", cone(&sim));
    press2(&mut sim, p);
    sim.seconds(1.0);
    // The penalty settles at InaccuracyStandAlt.
    assert!((cone(&sim) - 0.002).abs() < 1e-4, "scoped {}", cone(&sim));
    sim.intent(p).fire = true;
    sim.ticks(1);
    sim.intent(p).fire = false;
    sim.ticks(1);
    assert!(zoomed(&sim, p).is_none(), "the shot unzooms");
    assert_eq!(mode(&sim, p), 0);
    // Re-zooms when next_primary passes: 100 ticks after the shot.
    sim.ticks(97);
    assert!(zoomed(&sim, p).is_none());
    sim.ticks(2);
    assert_eq!(zoomed(&sim, p).map(|z| z.fov), Some(40.0));
}

#[test]
fn switching_away_unzooms() {
    let mut sim = sim();
    let p = shooter_with(&mut sim, AWP);
    press2(&mut sim, p);
    sim.ticks(1);
    assert!(zoomed(&sim, p).is_some());
    sim.intent(p).select = Some(2);
    sim.ticks(2);
    sim.intent(p).select = None;
    assert!(zoomed(&sim, p).is_none());
    assert!((speed(&sim, p) - 250.0).abs() < 1e-3, "knife speed");
    // Slot 1 holds the AK-47 first, then the AWP.
    for _ in 0..2 {
        sim.intent(p).select = Some(0);
        sim.ticks(1);
        sim.intent(p).select = None;
        sim.ticks(1);
    }
    assert_eq!(active_id(&sim, p), AWP);
    assert!(zoomed(&sim, p).is_none(), "drawn unzoomed");
}

// ---------------------------------------------------------------------------
// Silencers and burst (M16)

#[test]
fn silencers_block_both_attacks_and_change_accuracy() {
    // M16: M4A1 2.0 s, USP 3.0 s. Silenced: the *Alt keys (M4A1 Spread
    // 0.00054, USP 0.003).
    for (id, secs, spread) in [(M4A1, 2.0, 0.00054), (USP, 3.0, 0.003)] {
        let mut sim = sim();
        let p = shooter_with(&mut sim, id);
        press2(&mut sim, p);
        assert_eq!(mode(&sim, p), 1, "{id} silenced");
        sim.intent(p).fire = true;
        let shot = ticks_until_shot(&mut sim, p, 400).unwrap();
        let want = (secs / TICK_INTERVAL).ceil() as u32;
        assert!(
            (want - 1..=want).contains(&shot),
            "{id}: first shot {shot} ticks after, want {want}"
        );
        sim.intent(p).fire = false;
        sim.ticks(1);
        let w = active(&sim, p);
        let SpreadShape::Disc { spread: s, .. } = sim.app.world().get::<Hitscan>(w).unwrap().spread else {
            unreachable!()
        };
        assert!((s - spread).abs() < 1e-6, "{id} spread {s}");
        // And off again.
        sim.seconds(secs as f64);
        press2(&mut sim, p);
        assert_eq!(mode(&sim, p), 0, "{id} unsilenced");
    }
}

#[test]
fn glock_burst_fires_three_rounds_four_ticks_apart() {
    // M16: one press fires 3 rounds on ticks 0, 4 and 8; 3 rounds used.
    let mut sim = sim();
    let p = shooter_with(&mut sim, GLOCK);
    press2(&mut sim, p);
    assert_eq!(mode(&sim, p), 1);
    sim.seconds(0.4);
    let start = clip(&sim, p);
    sim.intent(p).fire = true;
    sim.ticks(1);
    assert_eq!(clip(&sim, p), start - 1);
    assert_eq!(ticks_until_shot(&mut sim, p, 10), Some(4));
    assert_eq!(ticks_until_shot(&mut sim, p, 10), Some(4));
    assert_eq!(clip(&sim, p), start - 3);
    // Held: no more.
    assert_eq!(ticks_until_shot(&mut sim, p, 60), None);
    // Single fire again after a toggle.
    sim.intent(p).fire = false;
    sim.seconds(0.4);
    press2(&mut sim, p);
    assert_eq!(mode(&sim, p), 0);
    sim.seconds(0.4);
    tap(&mut sim, p);
    assert_eq!(clip(&sim, p), start - 4);
    sim.seconds(0.5);
    assert_eq!(clip(&sim, p), start - 4);
}
