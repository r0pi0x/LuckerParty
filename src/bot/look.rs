//! Checking corners: where a walking bot looks when no enemy is in sight
//! (ours; CS:S's bot code isn't public). It looks at hiding spots as
//! they come into view, so it faces the places enemies could be rather
//! than straight down its path:
//!
//! - with the nav file's encounter data (version 9 meshes: de_nuke,
//!   cs_office), the spots listed for the way it is crossing its area, a
//!   little before the point where each becomes visible;
//! - otherwise (dust2's v16 mesh has none), hiding spots in areas its
//!   own area can see, ahead of it and in line of sight, nearest the
//!   walking direction first.
//!
//! A spot looked at (or passed) counts as checked for a while. With no
//! spot to check, the caller looks a few meters ahead along the route,
//! around the next corner.

use bevy::prelude::*;

use super::Bot;
use crate::map::nav::NavMesh;

/// Seconds a checked spot stays checked.
const CHECK_MEMORY: f64 = 10.0;
/// Seconds between looks for a new spot.
const SCAN_INTERVAL: f64 = 0.3;
/// Hiding spots this far from the eye are considered, m.
const SPOT_RANGE: (f32, f32) = (2.5, 25.0);
/// A spot counts as checked after the view is within this many degrees
/// of it for `CHECK_TIME` seconds, or after `GIVE_UP` seconds.
const CHECK_CONE_DEG: f32 = 12.0;
const CHECK_TIME: f64 = 0.3;
const GIVE_UP: f64 = 2.0;
/// Spots behind the walking direction (cosine below this) are left.
const AHEAD_COS: f32 = -0.2;
/// Line-of-sight tests per scan.
const RAYS: usize = 4;
/// Height looked at above a spot (a crouching or standing enemy's
/// chest), m.
pub const SPOT_LIFT: f32 = 1.0;

/// Corner-check memory of a bot.
#[derive(Clone, Debug, Default)]
pub(super) struct Corners {
    /// Spots checked, and when.
    checked: Vec<(Vec3, f64)>,
    /// The spot being looked at, since when, and since when on target.
    current: Option<(Vec3, f64, Option<f64>)>,
    next_scan: f64,
}

impl Corners {
    pub fn current(&self) -> Option<Vec3> {
        self.current.map(|c| c.0)
    }

    fn is_checked(&self, p: Vec3) -> bool {
        self.checked.iter().any(|(c, _)| c.distance_squared(p) < 0.01)
    }
}

/// The spot to look at (its floor position), if any: keeps the current
/// one until checked, else picks the next. `walk` is the walking
/// direction (flat, unit) or None standing; `looking` the view direction;
/// `los(a, b)` whether nothing blocks the line.
#[allow(clippy::too_many_arguments)]
pub(super) fn corner(
    bot: &mut Bot,
    nav: &NavMesh,
    eye: Vec3,
    feet: Vec3,
    walk: Option<Vec3>,
    looking: Vec3,
    now: f64,
    los: &dyn Fn(Vec3, Vec3) -> bool,
) -> Option<Vec3> {
    let c = &mut bot.corners;
    c.checked.retain(|(_, at)| now - at < CHECK_MEMORY);
    // Spots walked past are checked.
    if let Some(area) = bot.area {
        for s in &nav.areas[area].hiding {
            if s.pos.distance(feet) < 2.0 && !c.is_checked(s.pos) {
                c.checked.push((s.pos, now));
            }
        }
    }
    if let Some((p, since, on)) = c.current {
        let to = (p + Vec3::Y * SPOT_LIFT - eye).normalize_or_zero();
        let on_target = to.angle_between(looking).to_degrees() < CHECK_CONE_DEG;
        let on = match (on_target, on) {
            (true, None) => Some(now),
            (true, on) => on,
            (false, _) => None,
        };
        if on.is_some_and(|t| now - t >= CHECK_TIME) || now - since > GIVE_UP {
            c.checked.push((p, now));
            c.current = None;
        } else {
            c.current = Some((p, since, on));
            return Some(p);
        }
    }
    if now < c.next_scan {
        return None;
    }
    c.next_scan = now + SCAN_INTERVAL;
    let area = bot.area?;
    let ahead = |p: Vec3| {
        walk.is_none_or(|w| {
            let d = (p - feet).with_y(0.0).normalize_or_zero();
            d.dot(w) > AHEAD_COS
        })
    };
    // Encounter data for the way through this area.
    let next = bot
        .route_areas
        .get(bot.next)
        .copied()
        .filter(|n| Some(*n) != bot.area)
        .or_else(|| bot.route_areas.get(bot.next + 1).copied());
    if let (Some(prev), Some(next)) = (bot.prev_area, next)
        && let Some(enc) = nav.areas[area]
            .encounters
            .iter()
            .find(|e| e.from == prev && e.to == next)
    {
        // How far through the area: from where it came in to the next
        // route point.
        let exit = bot.route.get(bot.next).copied().unwrap_or(feet);
        let total = bot.entered.distance(exit).max(0.1);
        let t = (1.0 - feet.distance(exit) / total).clamp(0.0, 1.0);
        let pick = enc
            .spots
            .iter()
            .find(|(p, at)| *at <= t + 0.3 && !c.is_checked(*p) && p.distance(eye) > SPOT_RANGE.0)
            .map(|s| s.0);
        if let Some(p) = pick {
            c.current = Some((p, now, None));
            return Some(p);
        }
        return None;
    }
    // Hiding spots in view, ahead.
    let near: Vec<usize> = if nav.areas[area].visible.is_empty() {
        nav.within(area, SPOT_RANGE.1).into_iter().map(|a| a.0).collect()
    } else {
        std::iter::once(area)
            .chain(nav.areas[area].visible.iter().map(|v| v.0))
            .filter(|a| nav.areas[*a].center.distance(feet) < SPOT_RANGE.1 + 10.0)
            .collect()
    };
    let mut cands: Vec<(Vec3, f32)> = near
        .iter()
        .flat_map(|&a| nav.areas[a].hiding.iter().map(|s| s.pos))
        .filter(|p| {
            let d = p.distance(eye);
            (SPOT_RANGE.0..SPOT_RANGE.1).contains(&d) && ahead(*p) && !c.is_checked(*p)
        })
        .map(|p| {
            let dir = (p - feet).with_y(0.0).normalize_or_zero();
            let off = walk.map_or(0.0, |w| 1.0 - dir.dot(w));
            (p, off + p.distance(feet) / SPOT_RANGE.1)
        })
        .collect();
    cands.sort_by(|a, b| a.1.total_cmp(&b.1));
    for (p, _) in cands.into_iter().take(RAYS) {
        if los(eye, p + Vec3::Y * SPOT_LIFT) {
            c.current = Some((p, now, None));
            return Some(p);
        }
        // Out of sight from here: it'll come up again later.
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::path::tests::{area, link};
    use crate::map::nav::{Encounter, HidingSpot, Side, spot};

    fn mesh() -> NavMesh {
        let mut m = NavMesh {
            areas: vec![
                area(1, 0.0, 0.0, 10.0, 10.0, 0.0, 0),
                area(2, 10.0, 0.0, 20.0, 10.0, 0.0, 0),
                area(3, 20.0, 0.0, 30.0, 10.0, 0.0, 0),
            ],
            ..default()
        };
        link(&mut m, 0, 1, Side::MaxX);
        link(&mut m, 1, 2, Side::MaxX);
        m
    }

    fn walker(m: &NavMesh, at: Vec3) -> Bot {
        let mut bot = Bot::default();
        bot.area = m.area_at(at);
        bot.entered = at;
        bot
    }

    #[test]
    fn looks_at_spots_ahead_in_sight_until_checked() {
        let mut m = mesh();
        let ahead = Vec3::new(18.0, 0.0, 9.0);
        let behind = Vec3::new(1.0, 0.0, 9.0);
        let hidden = Vec3::new(16.0, 0.0, 1.0);
        for p in [ahead, behind, hidden] {
            let a = m.area_at(p + Vec3::Y * 0.1).unwrap();
            m.areas[a].hiding.push(HidingSpot {
                pos: p,
                flags: spot::IN_COVER,
            });
        }
        let feet = Vec3::new(6.0, 0.0, 5.0);
        let eye = feet + Vec3::Y * 1.6;
        let mut bot = walker(&m, feet);
        let los = |_: Vec3, b: Vec3| b.z > 5.0;
        let walk = Some(Vec3::X);
        let got = corner(&mut bot, &m, eye, feet, walk, Vec3::X, 0.0, &los);
        assert_eq!(got, Some(ahead), "the spot ahead in sight, not behind or hidden");
        // Keeps it while turning toward it...
        assert_eq!(corner(&mut bot, &m, eye, feet, walk, Vec3::X, 0.1, &los), Some(ahead));
        // ...and it's checked after looking at it a moment.
        let at = (ahead + Vec3::Y * SPOT_LIFT - eye).normalize();
        corner(&mut bot, &m, eye, feet, walk, at, 0.2, &los);
        corner(&mut bot, &m, eye, feet, walk, at, 0.6, &los);
        assert!(bot.corners.is_checked(ahead));
        assert_eq!(corner(&mut bot, &m, eye, feet, walk, at, 1.0, &los), None);
        // Remembered for a while only.
        bot.corners.next_scan = 0.0;
        assert_eq!(corner(&mut bot, &m, eye, feet, walk, at, 20.0, &los), Some(ahead));
    }

    #[test]
    fn encounter_spots_in_order_along_the_way() {
        let mut m = mesh();
        let first = Vec3::new(14.0, 0.0, 30.0);
        let second = Vec3::new(18.0, 0.0, -30.0);
        m.areas[1].encounters.push(Encounter {
            from: 0,
            to: 2,
            spots: vec![(first, 0.1), (second, 0.8)],
        });
        let feet = Vec3::new(10.5, 0.0, 5.0);
        let mut bot = walker(&m, feet);
        bot.prev_area = Some(0);
        bot.route = vec![Vec3::new(20.0, 0.0, 5.0)];
        bot.route_areas = vec![2];
        let los = |_: Vec3, _: Vec3| false;
        let eye = feet + Vec3::Y * 1.6;
        let walk = Some(Vec3::X);
        // Not visible yet by rays, but the data says it soon is: pre-aim.
        assert_eq!(corner(&mut bot, &m, eye, feet, walk, Vec3::X, 0.0, &los), Some(first));
        bot.corners.current = None;
        bot.corners.checked.push((first, 0.0));
        bot.corners.next_scan = 0.0;
        // The second only near the end of the area.
        assert_eq!(corner(&mut bot, &m, eye, feet, walk, Vec3::X, 0.5, &los), None);
        let late = Vec3::new(17.0, 0.0, 5.0);
        bot.corners.next_scan = 0.0;
        assert_eq!(
            corner(&mut bot, &m, late + Vec3::Y * 1.6, late, walk, Vec3::X, 1.0, &los),
            Some(second)
        );
    }
}
