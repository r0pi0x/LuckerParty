//! CS:S navigation meshes, from specs/cs_source/nav.md "Test cases". Read
//! from the local install; skipped without one.

use bevy::prelude::*;
use mashup::{
    games::cs_source::{self, nav},
    map::nav::{NavMesh, Via},
    mount::config::LocalConfig,
};

const U: f32 = 0.0254;

fn read(map: &str) -> Option<Vec<u8>> {
    let install = LocalConfig::load().ok()?.game_path(cs_source::GAME)?;
    if !install.join("cstrike").is_dir() {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    let mount = cs_source::mount::open(&install).ok()?;
    Some(mount.read(&format!("maps/{map}.nav")).expect("nav file"))
}

fn mesh(map: &str) -> Option<(NavMesh, nav::NavInfo)> {
    Some(nav::parse(&read(map)?).expect("parse"))
}

/// Source units (x, y, z) to engine meters.
fn engine(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, z, -y) * U
}

fn area<'a>(m: &'a NavMesh, id: u32) -> &'a mashup::map::nav::NavArea {
    &m.areas[m.index_of(id).unwrap()]
}

#[test]
fn dust2_parses_to_the_end() {
    let Some((m, info)) = mesh("de_dust2") else { return };
    assert_eq!((info.version, info.subversion), (16, 1));
    assert_eq!(info.trailing, 0);
    assert_eq!(m.areas.len(), 961);
    assert_eq!(m.ladders.len(), 0);
    assert_eq!(info.hiding_spots, 536);
    assert_eq!(info.encounters, 0);
    assert_eq!(
        m.places,
        [
            "Side",
            "CTSpawn",
            "TSpawn",
            "BombsiteA",
            "Middle",
            "BombsiteB",
            "Tunnel",
            "SideDoor",
            "DoubleDoors"
        ]
    );
    let links: usize = m.areas.iter().map(|a| a.links.len()).sum();
    assert_eq!(links, 3114);
    let tspawn = m.areas.iter().filter(|a| a.place == Some(2)).count();
    assert_eq!(tspawn, 201);
    // Area 2's links, N/E/S/W in file order.
    let a2 = area(&m, 2);
    let ids: Vec<u32> = a2.links.iter().map(|(i, _)| m.areas[*i].id).collect();
    assert_eq!(
        ids,
        [
            3669, 2771, 2787, 3682, 147, 592, 2568, 1092, 2773, 2774, 3982, 3983, 124, 23, 2532
        ]
    );
    // One-way: 8 -> 2096 east, no way back.
    let (i8, i2096) = (m.index_of(8).unwrap(), m.index_of(2096).unwrap());
    assert!(m.areas[i8].links.iter().any(|(t, _)| *t == i2096));
    assert!(!m.areas[i2096].links.iter().any(|(t, _)| *t == i8));
    assert!(area(&m, 4181).flags & 0x400 != 0);
}

#[test]
fn dust2_heights_and_area_queries() {
    let Some((m, _)) = mesh("de_dust2") else { return };
    let a2 = area(&m, 2);
    let h = |x: f32, y: f32| a2.height_at(x * U, -y * U) / U;
    assert!((h(1262.5, 987.5) - 1.7189506).abs() < 1e-3);
    assert!((h(1450.0, 800.0) - 3.8144591).abs() < 1e-3);
    assert!((h(0.0, 0.0) - 1.8456212).abs() < 1e-3);
    assert!(
        (a2.center.y / U - 1.0610091).abs() < 1e-3,
        "centre height is the NW/SE mean"
    );

    let at = |x, y, z| m.area_at(engine(x, y, z)).map(|i| m.areas[i].id);
    assert_eq!(at(275.0, 2225.0, -100.0), Some(3));
    assert_eq!(at(400.0, 2300.0, 150.0), Some(8));
    assert_eq!(at(275.0, 2225.0, 7.0), None);
    assert_eq!(at(1200.0, 1000.0, 40.0), Some(2));
    assert_eq!(at(1200.0, 1000.0, 200.0), None);
}

#[test]
fn dust2_stock_path() {
    let Some((m, _)) = mesh("de_dust2") else { return };
    let (a, b) = (m.index_of(3).unwrap(), m.index_of(5).unwrap());
    let (path, cost) = m.find_path(a, b).expect("path");
    let ids: Vec<u32> = path.iter().map(|(i, _)| m.areas[*i].id).collect();
    assert_eq!(
        ids,
        [
            3, 53, 4139, 4121, 10, 51, 240, 493, 785, 181, 77, 2691, 28, 137, 3948, 222, 2819, 1706, 1705, 1700, 1697,
            2861, 2687, 39, 3930, 3860, 3864, 3861, 3218, 252, 5
        ]
    );
    assert!((cost / U as f64 - 3676.23).abs() < 0.1, "cost {}", cost / U as f64);
    let (back, cost) = m.find_path(b, a).expect("path back");
    assert_eq!(back.len(), 31);
    assert!((cost / U as f64 - 3676.23).abs() < 0.1);
    // A route of world points from CT spawn to T spawn ends at the goal.
    let from = engine(275.0, 2225.0, -100.0);
    let to = m.areas[b].center;
    let route = m.route(from, to).expect("route");
    assert_eq!(*route.last().unwrap(), to);
    assert!(route.len() >= 30);
}

#[test]
fn train_ladders() {
    let Some((m, info)) = mesh("de_train") else { return };
    assert_eq!(info.version, 9);
    assert_eq!(info.trailing, 0);
    assert_eq!(m.areas.len(), 1335);
    assert_eq!(m.ladders.len(), 41);
    let l = &m.ladders[1];
    assert!((l.length / U - 94.0).abs() < 1e-3);
    assert!((l.top - engine(285.0, 546.0, -78.13804)).length() < 1e-3);
    // Ladder 1 (id 1) goes up from its bottom area 948 to 67 and 512.
    let bottom = m.index_of(948).unwrap();
    let ups: Vec<u32> = m.areas[bottom]
        .links
        .iter()
        .filter(|(_, v)| *v == Via::LadderUp(0))
        .map(|(t, _)| m.areas[*t].id)
        .collect();
    assert_eq!(ups, [67, 512]);
}
