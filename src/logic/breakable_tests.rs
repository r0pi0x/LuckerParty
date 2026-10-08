//! Test cases from specs/source/breakables.md on the logic world.

use super::*;
use crate::logic::world::{NoCollision, Player};
use crate::map::{MapBrush, MapHull};

const DT: f32 = 0.015;

fn hull(lo: Vec3, hi: Vec3) -> MapHull {
    let b = MapBrush::from_box(lo, hi);
    let points = (0..8)
        .map(|i| {
            Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            )
        })
        .collect();
    MapHull {
        planes: b.planes,
        points,
    }
}

fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn world() -> LogicWorld {
    let mut w = LogicWorld::new(DT);
    w.record = true;
    w
}

fn breakable(w: &mut LogicWorld, pairs: &[(&str, &str)], size: Vec3) -> EntId {
    let mut all = vec![("classname", "func_breakable")];
    all.extend_from_slice(pairs);
    let id = w.spawn(&kv(&all), vec![hull(-size / 2.0, size / 2.0)]);
    w.activate();
    id
}

fn health(w: &LogicWorld, id: EntId) -> i32 {
    match &w.get(id).unwrap().class {
        Class::Breakable(b) => b.health,
        _ => panic!(),
    }
}

fn gibs(w: &LogicWorld) -> usize {
    w.effects
        .iter()
        .map(|e| match e {
            Effect::Gibs { pieces, .. } => pieces.len(),
            _ => 0,
        })
        .sum()
}

fn sounds(w: &LogicWorld) -> Vec<String> {
    w.effects
        .iter()
        .filter_map(|e| match e {
            Effect::Sound { entry, .. } => Some(entry.clone()),
            _ => None,
        })
        .collect()
}

fn player(at: Vec3, velocity: Vec3) -> Player {
    let mut p = Player::new(Entity::from_raw_u32(1).unwrap(), at);
    p.velocity = velocity;
    p
}

#[test]
fn nuke_vent_breaks_from_one_bullet() {
    // Metal, health 1, a 64x8x32 brush: one bullet; 6 MetalChunks gibs.
    let mut w = world();
    let vent = breakable(
        &mut w,
        &[("material", "2"), ("health", "1"), ("targetname", "vent")],
        Vec3::new(64.0, 8.0, 32.0),
    );
    assert!(w.mover_solid(vent).is_some(), "solid before");
    assert!(w.shootable(vent));
    w.damage(vent, 26.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    assert!(w.mover_solid(vent).is_none(), "non-solid at once");
    assert!(!w.shootable(vent));
    assert_eq!(gibs(&w), 6);
    assert!(
        w.effects
            .iter()
            .any(|e| matches!(e, Effect::Gibs { set, glass: false, .. } if set == "MetalChunks"))
    );
    assert_eq!(sounds(&w), vec!["Breakable.Metal"]);
    assert!(w.find("vent").is_none(), "name cleared");
    assert!(w.fired.iter().any(|(_, e, o)| *e == vent && o == "OnBreak"));
    // Deleted 7 ticks later (in tick 7's frame).
    for _ in 0..7 {
        w.frame(&NoCollision);
        assert!(w.get(vent).is_some());
    }
    w.frame(&NoCollision);
    assert!(w.get(vent).is_none(), "removed after 0.1 s");
}

#[test]
fn knife_breaks_a_vent_too() {
    let mut w = world();
    let vent = breakable(
        &mut w,
        &[("material", "2"), ("health", "1")],
        Vec3::new(64.0, 8.0, 32.0),
    );
    w.damage(vent, 15.0, DamageKind::Melee, None, Vec3::ZERO, Vec3::X);
    assert!(w.mover_solid(vent).is_none());
}

#[test]
fn damage_scaling_and_truncation() {
    let mut w = world();
    let wood = breakable(&mut w, &[("material", "1"), ("health", "50")], Vec3::splat(32.0));
    w.damage(wood, 36.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    assert_eq!(health(&w, wood), 32);
    assert_eq!(sounds(&w), vec!["Breakable.MatWood"]);
    w.damage(wood, 36.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    assert_eq!(health(&w, wood), 14);
    w.damage(wood, 36.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    assert!(w.mover_solid(wood).is_none(), "third shot breaks it");

    let mut w = world();
    let wood = breakable(&mut w, &[("material", "1"), ("health", "50")], Vec3::splat(32.0));
    w.damage(wood, 35.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    assert_eq!(health(&w, wood), 32, "50 - 17.5 truncated");

    let mut w = world();
    let wood = breakable(&mut w, &[("material", "1"), ("health", "50")], Vec3::splat(32.0));
    w.damage(wood, 34.0, DamageKind::Melee, None, Vec3::ZERO, Vec3::X);
    assert!(w.mover_solid(wood).is_none(), "club 34: 50 - 51 breaks");

    let mut w = world();
    let wood = breakable(&mut w, &[("material", "1"), ("health", "50")], Vec3::splat(32.0));
    w.damage(wood, 40.0, DamageKind::Blast, None, Vec3::ZERO, Vec3::X);
    assert!(w.mover_solid(wood).is_none(), "50 - 50 = 0 breaks");

    let mut w = world();
    let wood = breakable(
        &mut w,
        &[("material", "1"), ("health", "50"), ("minhealthdmg", "20")],
        Vec3::splat(32.0),
    );
    w.damage(wood, 18.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    assert_eq!(health(&w, wood), 50, "raw 18 < 20 ignored");
    w.damage(wood, 36.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    assert_eq!(health(&w, wood), 32);
}

#[test]
fn undamageable_ones_break_only_by_input() {
    let mut w = world();
    let glass = breakable(&mut w, &[("material", "0"), ("health", "0")], Vec3::splat(32.0));
    w.damage(glass, 100.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    assert!(w.mover_solid(glass).is_some(), "not damageable");
    let trig = breakable(
        &mut w,
        &[("material", "1"), ("health", "10"), ("spawnflags", "1")],
        Vec3::splat(32.0),
    );
    w.damage(trig, 100.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    assert!(w.mover_solid(trig).is_some());
    w.deliver(Who::Ent(trig), "RemoveHealth", Value::Int(10), None, None);
    assert!(w.mover_solid(trig).is_none(), "RemoveHealth 10 breaks it");
    w.deliver(Who::Ent(glass), "Break", Value::Void, None, None);
    assert!(w.mover_solid(glass).is_none(), "Break works");
}

#[test]
fn gib_counts() {
    assert_eq!(gib_count(Vec3::splat(128.0), 0), 15);
    assert_eq!(gib_count(Vec3::splat(128.0), 3), 7);
    assert_eq!(gib_count(Vec3::splat(128.0), 1), 0);
    assert_eq!(gib_count(Vec3::splat(8.0), 3), 0);
}

#[test]
fn directed_gibs() {
    let mut w = world();
    let b = breakable(
        &mut w,
        &[
            ("material", "1"),
            ("health", "1"),
            ("explosion", "2"),
            ("gibdir", "0 90 0"),
        ],
        Vec3::splat(64.0),
    );
    w.damage(b, 26.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    let Some(Effect::Gibs { pieces, .. }) = w.effects.iter().find(|e| matches!(e, Effect::Gibs { .. })) else {
        panic!()
    };
    assert!(!pieces.is_empty());
    for g in pieces {
        assert!(
            g.velocity.y >= 100.0 - 1e-3 && g.velocity.y <= 300.0 + 1e-3,
            "{}",
            g.velocity
        );
        assert!(g.velocity.x.abs() <= 100.0 + 1e-3 && (0.0..=100.0).contains(&g.velocity.z));
        assert!(g.life >= 2.5 && g.life <= 3.5);
    }
}

#[test]
fn breaks_on_touch_when_fast_enough() {
    let mut w = world();
    let b = breakable(
        &mut w,
        &[("material", "0"), ("health", "1"), ("spawnflags", "2")],
        Vec3::new(8.0, 64.0, 64.0),
    );
    w.players = vec![player(Vec3::new(-20.0, 0.0, -30.0), Vec3::new(80.0, 0.0, 0.0))];
    w.frame(&NoCollision);
    assert!(w.mover_solid(b).is_some(), "80 units/s: nothing");
    w.players[0].velocity = Vec3::new(250.0, 0.0, 0.0);
    w.effects.clear();
    w.frame(&NoCollision);
    assert!(w.mover_solid(b).is_none(), "250 units/s breaks it");
    assert!(
        w.effects
            .iter()
            .any(|e| matches!(e, Effect::Damage { amount, .. } if (amount - 0.625).abs() < 1e-4))
    );
}

/// cs_office window *1: 120 x 96 units, 10 x 8 panes.
fn office_window(w: &mut LogicWorld) -> EntId {
    let id = w.spawn(
        &kv(&[
            ("classname", "func_breakable_surf"),
            ("health", "1"),
            ("fragility", "100"),
            ("surfacetype", "0"),
            ("lowerleft", "-508 -344 -148"),
            ("lowerright", "-628 -344 -148"),
            ("upperleft", "-508 -344 -52"),
            ("upperright", "-628 -344 -52"),
        ]),
        vec![hull(
            Vec3::new(-628.0, -345.0, -148.0),
            Vec3::new(-508.0, -343.0, -52.0),
        )],
    );
    w.activate();
    id
}

/// A point in pane (c, r) at fractions (fx, fy) of the office window.
fn office_point(c: f32, r: f32) -> Vec3 {
    Vec3::new(-508.0 - 12.0 * c, -344.0, -148.0 + 12.0 * r)
}

#[test]
fn window_first_hit_breaks_it_and_a_pane() {
    let mut w = world();
    let win = office_window(&mut w);
    let g = w.window(win).unwrap();
    assert_eq!((g.cols, g.rows), (10, 8));
    assert!((g.u.length() - 12.0).abs() < 1e-4 && (g.v.length() - 12.0).abs() < 1e-4);
    assert!(w.mover_solid(win).is_some());
    w.damage(win, 26.0, DamageKind::Bullet, None, office_point(4.5, 3.5), Vec3::NEG_Y);
    let g = w.window(win).unwrap();
    assert!(g.window_broken);
    assert!(g.broken[g.index(4, 3)]);
    assert!(w.mover_solid(win).is_none(), "non-solid after the first hit");
    assert!(w.shootable(win), "panes still hit by shots");
    assert!(w.fired.iter().any(|(_, e, o)| *e == win && o == "OnBreak"));
    assert!(sounds(&w).contains(&"Glass.Break".to_string()));
    assert!(w.effects.iter().any(|e| matches!(e, Effect::PaneShatter { .. })));
}

/// A hit that shatters the pane it hits sends the glass impact (at the
/// hit, the normal facing the shot: impact_effects.md section 9); a later
/// hit on that broken pane sends none.
#[test]
fn a_hit_that_shatters_its_pane_sends_the_glass_impact() {
    let impacts = |w: &LogicWorld| -> Vec<(Vec3, Vec3)> {
        w.effects
            .iter()
            .filter_map(|e| match e {
                Effect::GlassImpact { at, normal } => Some((*at, *normal)),
                _ => None,
            })
            .collect()
    };
    let mut w = world();
    let win = office_window(&mut w);
    let at = office_point(4.5, 3.5);
    w.damage(win, 26.0, DamageKind::Bullet, None, at, Vec3::Y);
    let hits = impacts(&w);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].0.distance(at) < 1e-3);
    assert!((hits[0].1 - Vec3::NEG_Y).length() < 1e-3, "facing the shot: {}", hits[0].1);
    w.effects.clear();
    w.damage(win, 26.0, DamageKind::Bullet, None, at, Vec3::NEG_Y);
    assert!(impacts(&w).is_empty(), "the pane was already broken");
}

#[test]
fn edge_hits_take_the_neighbour() {
    let mut w = world();
    let win = office_window(&mut w);
    w.damage(win, 26.0, DamageKind::Bullet, None, office_point(4.9, 3.5), Vec3::Y);
    let g = w.window(win).unwrap();
    assert!(g.broken[g.index(4, 3)] && g.broken[g.index(5, 3)]);
    assert!(!g.broken[g.index(3, 3)]);
}

#[test]
fn support_values() {
    let window = || {
        Window::new(
            Vec3::ZERO,
            Vec3::new(36.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 36.0),
            Vec3::ZERO,
            Vec3::ZERO,
            false,
            100.0,
        )
    };
    let mut g = window();
    assert!(g.supports().iter().all(|s| (s - 6.76 / 6.75).abs() < 1e-5));
    // Bottom-left pane with every inside neighbour broken: 4.51/6.75.
    for (c, r) in [(1, 0), (0, 1), (1, 1)] {
        let i = g.index(c, r);
        g.broken[i] = true;
        g.support[i] = 0.0;
    }
    let s = g.supports()[g.index(0, 0)];
    assert!((s - 4.51 / 6.75).abs() < 1e-5, "{s}");
    // An interior pane with all 8 neighbours broken collapses.
    let mut g = window();
    let keep = g.index(1, 1);
    for i in 0..9 {
        if i != keep {
            g.broken[i] = true;
            g.support[i] = 0.0;
        }
    }
    let s = g.supports()[keep];
    assert!(s < PANE_BREAK_SUPPORT, "{s}");
}

#[test]
fn shatter_input_and_cascade() {
    let mut w = world();
    let win = office_window(&mut w);
    w.deliver(
        Who::Ent(win),
        "Shatter",
        Value::Vector(Vec3::new(0.5, 0.5, 1000.0)),
        None,
        None,
    );
    assert_eq!(w.window(win).unwrap().broken_count(), 80, "radius covers the window");
    assert!(!w.shootable(win), "nothing left to hit");

    // Every pane but one interior one: it collapses on the next pass.
    let mut w = world();
    let win = office_window(&mut w);
    let keep = w.window(win).unwrap().index(5, 4);
    for i in 0..80 {
        if i != keep {
            shatter(&mut w, win, (i % 10) as i32, (i / 10) as i32, Vec3::ZERO);
        }
    }
    w.frame(&NoCollision);
    w.frame(&NoCollision);
    assert_eq!(w.window(win).unwrap().broken_count(), 80, "the lone pane fell");
}

#[test]
fn walking_through_a_broken_window_shatters_panes() {
    let mut w = world();
    let win = office_window(&mut w);
    w.damage(win, 26.0, DamageKind::Bullet, None, office_point(1.5, 7.5), Vec3::Y);
    let before = w.window(win).unwrap().broken_count();
    // A 32-wide, 72-tall player box standing in the window plane.
    w.players = vec![player(office_point(5.0, 0.0), Vec3::new(0.0, 250.0, 0.0))];
    w.frame(&NoCollision);
    let g = w.window(win).unwrap();
    assert!(g.broken_count() >= before + 3 * 6, "{} -> {}", before, g.broken_count());
    assert!(g.broken[g.index(5, 0)]);
}

/// The office window's front faces +Y (its corners' normal); a first hit
/// from the back turns the panes to that side: counted from the
/// attacker's lower left, the normal toward them (spec "Window break" 2).
#[test]
fn window_faces_the_attacked_side() {
    let mut w = world();
    let win = office_window(&mut w);
    assert!((w.window(win).unwrap().normal - Vec3::Y).length() < 1e-5);
    // From the back (travelling +Y): the pane hit in column 1 from the
    // front's left is column 8 from the back's.
    w.damage(win, 26.0, DamageKind::Bullet, None, office_point(1.5, 3.5), Vec3::Y);
    let g = w.window(win).unwrap();
    assert!((g.normal - Vec3::NEG_Y).length() < 1e-5, "{}", g.normal);
    assert!(g.broken[g.index(8, 3)] && !g.broken[g.index(1, 3)]);
    // Its reference corner: the front's lower right, 1 unit toward the
    // attacker.
    assert!((g.corner - Vec3::new(-628.0, -345.0, -148.0)).length() < 1e-4, "{}", g.corner);
    // From the front nothing turns.
    let mut w = world();
    let win = office_window(&mut w);
    w.damage(win, 26.0, DamageKind::Bullet, None, office_point(1.5, 3.5), Vec3::NEG_Y);
    let g = w.window(win).unwrap();
    assert!((g.normal - Vec3::Y).length() < 1e-5 && g.broken[g.index(1, 3)]);
}

/// A blast shatters a whole window as one large-shard burst pushed by
/// 3000 × damage along the blast (spec test case: 100 damage → 300000).
#[test]
fn a_blast_shatters_the_whole_window_at_once() {
    let mut w = world();
    let win = office_window(&mut w);
    w.damage(win, 100.0, DamageKind::Blast, None, office_point(5.0, 4.0), Vec3::NEG_Y);
    let g = w.window(win).unwrap();
    assert_eq!(g.broken_count(), 80);
    let bursts: Vec<_> = w
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::PaneShatter {
                size, velocity, shard, ..
            } => Some((*size, *velocity, *shard)),
            _ => None,
        })
        .collect();
    assert_eq!(bursts.len(), 1, "{bursts:?}");
    let (size, velocity, shard) = bursts[0];
    assert!((size - Vec2::new(120.0, 96.0)).length() < 1e-3);
    assert_eq!(shard, LARGE_SHARD);
    // The push (units/s, 1 % of the force) along the blast.
    assert!((velocity - Vec3::NEG_Y * 3000.0).length() < 1e-2, "{velocity}");

    // With a fifth already gone: one burst per column's unbroken run.
    let mut w = world();
    let win = office_window(&mut w);
    for c in 0..10 {
        for r in [3, 4] {
            shatter(&mut w, win, c, r, Vec3::ZERO);
        }
    }
    w.effects.clear();
    w.damage(win, 100.0, DamageKind::Blast, None, office_point(5.0, 4.0), Vec3::NEG_Y);
    let n = w.effects.iter().filter(|e| matches!(e, Effect::PaneShatter { .. })).count();
    assert_eq!(n, 20, "two runs in each of 10 columns");
}

/// Panes that lose their support collapse; half of them drop a falling
/// piece at their corner in the window's plane.
#[test]
fn collapsing_panes_drop_falling_pieces() {
    let mut w = world();
    let win = office_window(&mut w);
    // Keep every third pane both ways below the top row (the top edge
    // holds a pane there up): each has no neighbour left.
    for r in 0..8 {
        for c in 0..10 {
            if !(r % 3 == 1 && c % 3 == 1 && r < 6) {
                shatter(&mut w, win, c, r, Vec3::ZERO);
            }
        }
    }
    w.effects.clear();
    w.frame(&NoCollision);
    assert_eq!(w.window(win).unwrap().broken_count(), 80, "every lone pane fell");
    let falls: Vec<_> = w
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::PaneFall { at, axes, body, spin, .. } => Some((*at, *axes, *body, *spin)),
            _ => None,
        })
        .collect();
    // 6 lone panes, about half drop a piece.
    assert!(!falls.is_empty() && falls.len() < 6, "{}", falls.len());
    for (at, axes, body, spin) in falls {
        // A pane corner: on the window's 12-unit grid from its lower left.
        let c = (-508.0 - at.x) / 12.0;
        let r = (at.z + 148.0) / 12.0;
        assert!((c - c.round()).abs() < 1e-3 && (r - r.round()).abs() < 1e-3, "{at}");
        assert!((at.y + 344.0).abs() < 1e-3);
        assert!((axes[2] - Vec3::Y).length() < 1e-5);
        assert!(body < crate::map::breakables::PANE_PIECE_BODIES);
        assert!(spin.abs().max_element() <= PANE_PIECE_SPIN);
    }
}

/// Break on pressure (flag 4): a player standing on it plays its damage
/// sound, and it breaks `PressureDelay` later with them as breaker.
#[test]
fn breaks_under_pressure_after_its_delay() {
    let mut w = world();
    let b = breakable(
        &mut w,
        &[("material", "1"), ("health", "50"), ("spawnflags", "4"), ("PressureDelay", "0.3")],
        Vec3::new(64.0, 64.0, 8.0),
    );
    let mut p = player(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO);
    w.frame(&NoCollision);
    assert!(matches!(&w.get(b).unwrap().class, Class::Breakable(x) if !x.broken));
    p.ground = Some(b);
    p.on_ground = true;
    w.players = vec![p];
    w.frame(&NoCollision);
    assert!(sounds(&w).contains(&"Breakable.MatWood".to_string()), "{:?}", sounds(&w));
    // 0.3 s = 20 ticks.
    for _ in 0..18 {
        w.frame(&NoCollision);
    }
    assert!(matches!(&w.get(b).unwrap().class, Class::Breakable(x) if !x.broken));
    w.frame(&NoCollision);
    w.frame(&NoCollision);
    assert!(w.fired.iter().any(|(_, e, o)| *e == b && o == "OnBreak"));
}

/// explodemagnitude: an explosion at its centre when it breaks (radius
/// 2.5 × magnitude without ExplodeRadius), the breaker its attacker.
#[test]
fn explodes_when_it_breaks() {
    let mut w = world();
    let b = breakable(
        &mut w,
        &[("material", "1"), ("health", "10"), ("explodemagnitude", "120")],
        Vec3::splat(32.0),
    );
    let who = Who::Player(Entity::from_raw_u32(7).unwrap());
    w.damage(b, 100.0, DamageKind::Bullet, Some(who), Vec3::ZERO, Vec3::X);
    let x = w.effects.iter().find_map(|e| match e {
        Effect::Explosion {
            at,
            damage,
            radius,
            attacker,
            inflictor,
        } => Some((*at, *damage, *radius, *attacker, *inflictor)),
        _ => None,
    });
    assert_eq!(x, Some((Vec3::ZERO, 120.0, 300.0, Some(who), b)));
    let mut w = world();
    let b = breakable(
        &mut w,
        &[("material", "1"), ("health", "10"), ("ExplodeDamage", "50"), ("ExplodeRadius", "90")],
        Vec3::splat(32.0),
    );
    w.deliver(Who::Ent(b), "Break", Value::Void, None, None);
    assert!(
        w.effects
            .iter()
            .any(|e| matches!(e, Effect::Explosion { damage, radius, .. } if *damage == 50.0 && *radius == 90.0))
    );
}

/// What physics impacts do: glass counts 2 kg on the glass table; flag
/// 1024 takes none; flag 512 breaks on the first impact; a crush hit
/// breaks a window entity only, and a broken window takes none.
#[test]
fn physics_impact_rules() {
    let mut w = world();
    let glass = breakable(&mut w, &[("material", "0"), ("health", "1")], Vec3::new(64.0, 2.0, 64.0));
    let (table, scale, mass, instant) = w.breakable_impact(glass).unwrap();
    assert_eq!(
        (table, scale, mass, instant),
        (crate::logic::prop_damage::GLASS_TABLE, 1.0, 2.0, false)
    );
    let wood = breakable(
        &mut w,
        &[("material", "1"), ("health", "10"), ("spawnflags", "512"), ("physdamagescale", "0.5")],
        Vec3::splat(16.0),
    );
    let (_, scale, _, instant) = w.breakable_impact(wood).unwrap();
    assert_eq!((scale, instant), (0.5, true));
    let none = breakable(
        &mut w,
        &[("material", "1"), ("health", "10"), ("spawnflags", "1024")],
        Vec3::splat(16.0),
    );
    assert!(w.breakable_impact(none).is_none());
    let win = office_window(&mut w);
    assert!(w.breakable_impact(win).is_some());
    w.damage(win, 5.0, DamageKind::Crush, None, office_point(1.0, 1.0), Vec3::Y);
    let g = w.window(win).unwrap();
    assert!(g.window_broken && g.broken_count() == 0, "crush breaks the window entity only");
    assert!(w.breakable_impact(win).is_none());
}
