//! The CS:S HUD laid out from the install's files lands where the real
//! game draws it: each part's ink rectangle at 1920x1080, 1280x720 and
//! 1024x768 against the one measured on CS:S captures (`refcmp hudcmp`,
//! docs/OBSERVABILITY.md "Comparing the HUD"; the numbers are the
//! reference column of its report). Skipped without an install.

use std::sync::Arc;

use bevy::prelude::*;
use mashup::{
    client::{
        game_hud::{self, Drawn, HudValues, PanelKind, Part},
        hud_text::{self, ink_rect},
        weapon_select::{self, Row},
    },
    games::{self, cs_source},
    map::hud::GameHud,
    mount::config::LocalConfig,
};

fn hud() -> Option<Arc<GameHud>> {
    let installed = LocalConfig::load()
        .ok()?
        .game_path(cs_source::GAME)
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    let map = games::load_map("cs_source:de_dust2").expect("load de_dust2");
    Some(map.hud.expect("CS:S has a HUD"))
}

/// A part to check: what, and its ink rectangle [x0, y0, x1, y1) in CS:S.
enum Check {
    Text(Part),
    /// The ammo panel's clip, bar and reserve together.
    Ammo,
}

/// One capture's state and its measured parts.
struct Capture {
    size: (f32, f32),
    clock: f32,
    parts: &'static [(&'static str, [f32; 4])],
    /// Pixels each edge may be off by.
    tolerance: f32,
}

fn check_of(name: &str) -> Check {
    use PanelKind::*;
    if name == "ammo_icon" {
        return Check::Text(Part::Icon(Ammo));
    }
    let kind = |n: &str| match n {
        "health" => Health,
        "armor" => Armor,
        "timer" => Timer,
        "money" => Account,
        _ => Ammo,
    };
    match name.split_once('_') {
        Some((p, "icon")) => Check::Text(Part::Icon(kind(p))),
        Some((p, "digits")) => Check::Text(Part::Digits(kind(p))),
        _ => Check::Ammo,
    }
}

/// The weapon-selection captures (`select`: the selection open on slot 2,
/// the USP held, 100 health, no armour, $800, 12 | 100 ammo).
const CAPTURES: [Capture; 3] = [
    Capture {
        size: (1920.0, 1080.0),
        clock: 7.0 * 60.0 + 52.0,
        parts: &[
            ("health_icon", [36.0, 1011.0, 71.0, 1045.0]),
            ("health_digits", [96.0, 1011.0, 179.0, 1045.0]),
            ("armor_icon", [352.0, 1012.0, 385.0, 1045.0]),
            ("armor_digits", [467.0, 1011.0, 492.0, 1045.0]),
            ("timer_icon", [915.0, 1006.0, 947.0, 1045.0]),
            ("timer_digits", [991.0, 1011.0, 1097.0, 1045.0]),
            ("money_icon", [1664.0, 934.0, 1689.0, 978.0]),
            ("money_digits", [1782.0, 939.0, 1865.0, 973.0]),
            ("ammo", [1614.0, 1009.0, 1791.0, 1054.0]),
            ("ammo_icon", [1821.0, 1020.0, 1869.0, 1051.0]),
        ],
        tolerance: 1.0,
    },
    Capture {
        size: (1024.0, 768.0),
        clock: 7.0 * 60.0 + 42.0,
        parts: &[
            ("health_icon", [24.0, 719.0, 48.0, 743.0]),
            ("health_digits", [68.0, 719.0, 125.0, 743.0]),
            ("armor_icon", [248.0, 720.0, 271.0, 743.0]),
            ("armor_digits", [330.0, 720.0, 347.0, 743.0]),
            ("timer_icon", [480.0, 716.0, 502.0, 743.0]),
            ("timer_digits", [535.0, 719.0, 608.0, 743.0]),
            ("money_icon", [842.0, 664.0, 859.0, 694.0]),
            ("money_digits", [928.0, 668.0, 985.0, 691.0]),
            ("ammo", [805.0, 717.0, 930.0, 749.0]),
            ("ammo_icon", [954.0, 726.0, 988.0, 747.0]),
        ],
        tolerance: 1.0,
    },
    // At 720 lines the reference's pale glyphs over a bright floor measure
    // a pixel or two thin: digits only, 2 pixels either way.
    Capture {
        size: (1280.0, 720.0),
        clock: 7.0 * 60.0 + 57.0,
        parts: &[
            ("health_digits", [64.0, 675.0, 120.0, 698.0]),
            ("armor_digits", [313.0, 676.0, 330.0, 698.0]),
            ("timer_digits", [662.0, 675.0, 734.0, 698.0]),
            ("money_digits", [1186.0, 627.0, 1242.0, 650.0]),
            ("ammo", [1077.0, 673.0, 1195.0, 703.0]),
        ],
        tolerance: 2.0,
    },
];

fn close(name: &str, size: (f32, f32), ours: Rect, theirs: [f32; 4], tolerance: f32) -> Option<String> {
    let o = [ours.min.x, ours.min.y, ours.max.x, ours.max.y];
    let off = o.iter().zip(theirs).any(|(a, b)| (a - b).abs() > tolerance);
    off.then(|| format!("{}x{} {name}: ours {o:?}, CS:S {theirs:?}", size.0, size.1))
}

#[test]
fn hud_panels_land_where_css_draws_them() {
    let Some(hud) = hud() else { return };
    let mut wrong = Vec::new();
    for c in &CAPTURES {
        let (w, h) = c.size;
        let values = HudValues {
            health: Some((100.0, Color::WHITE)),
            armor: Some((0.0, false)),
            ammo: Some((12, 100)),
            ammo_icon: Some("ammo_45"),
            money: Some(800),
            clock: Some(c.clock),
        };
        let drawn = game_hud::layout(&hud, &|n| hud_text::game_font(&hud, n, h), w, h, &values);
        let find = |part: Part| drawn.iter().find(|(p, _)| *p == part).map(|(_, d)| d);
        let ink = |part: Part| match find(part) {
            Some(Drawn::Text(t)) => ink_rect(t),
            Some(Drawn::Box(r, ..)) => Some(*r),
            None => None,
        };
        for (name, theirs) in c.parts {
            let ours = match check_of(name) {
                Check::Text(part) => ink(part),
                Check::Ammo => [Part::Digits(PanelKind::Ammo), Part::Bar, Part::Digits2(PanelKind::Ammo)]
                    .into_iter()
                    .map(ink)
                    .collect::<Option<Vec<Rect>>>()
                    .and_then(|r| r.into_iter().reduce(|a, b| a.union(b))),
            };
            let Some(ours) = ours else {
                wrong.push(format!("{w}x{h} {name}: not drawn"));
                continue;
            };
            // The ammo glyph's thin strokes blur in the JPEG captures.
            let tolerance = if *name == "ammo_icon" {
                c.tolerance.max(2.0)
            } else {
                c.tolerance
            };
            wrong.extend(close(name, c.size, ours, *theirs, tolerance));
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

/// The weapon selection open on slot 2 with the USP held: the large box
/// and the USP's icon.
#[test]
fn weapon_selection_lands_where_css_draws_it() {
    let Some(hud) = hud() else { return };
    // (window, box, icon ink) measured on the captures.
    let captures = [
        (
            (1920.0, 1080.0),
            [685.0, 36.0, 928.0, 216.0],
            [729.0, 67.0, 861.0, 158.0],
        ),
        (
            (1280.0, 720.0),
            [457.0, 24.0, 619.0, 144.0],
            [486.0, 44.0, 576.0, 106.0],
        ),
        (
            (1024.0, 768.0),
            [318.0, 25.0, 490.0, 153.0],
            [349.0, 48.0, 443.0, 113.0],
        ),
    ];
    let row = Row {
        filled: vec![0, 1, 2],
        highlighted: (1, "usp", true),
    };
    let mut wrong = Vec::new();
    for ((w, h), large_box, icon) in captures {
        let drawn = weapon_select::selection_layout(&hud, &|n| hud_text::game_font(&hud, n, h), w, h, &row, 1.0);
        let large = drawn.iter().find_map(|d| match d {
            Drawn::Box(r, ..) => Some(*r),
            Drawn::Text(_) => None,
        });
        // The icon: the text in the weapon font.
        let usp = drawn.iter().find_map(|d| match d {
            Drawn::Text(t) if t.text == "A" => ink_rect(t),
            _ => None,
        });
        match large {
            Some(r) => wrong.extend(close("large box", (w, h), r, large_box, 0.0)),
            None => wrong.push(format!("{w}x{h}: no large box")),
        }
        match usp {
            Some(r) => wrong.extend(close("USP icon", (w, h), r, icon, 2.0)),
            None => wrong.push(format!("{w}x{h}: no USP icon")),
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

/// The pickup history after spawning with the USP and knife and getting an
/// AK-47 (1920x1080): newest at the bottom, right-aligned.
#[test]
fn pickup_history_lands_where_css_draws_it() {
    let Some(hud) = hud() else { return };
    let (w, h) = (1920.0, 1080.0);
    let items = [("usp", 1.0), ("knife", 1.0), ("ak47", 1.0)];
    let drawn = weapon_select::history_layout(&hud, &|n| hud_text::game_font(&hud, n, h), w, h, &items);
    let inks: Vec<Rect> = drawn
        .iter()
        .filter_map(|d| match d {
            Drawn::Text(t) => ink_rect(t),
            Drawn::Box(..) => None,
        })
        .collect();
    // Newest first in the drawn list; CS:S's ink measured on the capture
    // over crates of a similar colour (a few pixels uncertain; the
    // bottoms, which place the rows, agree within one).
    let theirs = [
        [1710.0, 696.0, 1876.0, 756.0],
        [1712.0, 601.0, 1873.0, 632.0],
        [1765.0, 480.0, 1876.0, 554.0],
    ];
    assert_eq!(inks.len(), 3);
    let wrong: Vec<String> = inks
        .iter()
        .zip(theirs)
        .filter_map(|(ours, theirs)| close("history", (w, h), *ours, theirs, 5.0))
        .chain(
            inks.iter()
                .zip(theirs)
                .filter(|(ours, theirs)| (ours.max.y - theirs[3]).abs() > 1.0)
                .map(|(ours, theirs)| format!("history row bottom: ours {}, CS:S {}", ours.max.y, theirs[3])),
        )
        .collect();
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}
