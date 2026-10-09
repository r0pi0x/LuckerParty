//! `refcmp hudcmp`: the HUD compared panel by panel between CS:S captures
//! and ours (docs/OBSERVABILITY.md, "Comparing the HUD").
//!
//! Each panel part (digits, icons, the selection's numbers, icon and name,
//! the pickup history) is a region in the virtual 480-line screen anchored
//! like the HUD (left, centre or right edge). In both images the region's
//! HUD-coloured pixels (orange or yellow over anything) become a weight
//! map; the offset of ours against the reference is the shift that best
//! correlates the two maps, and their ink boxes give the sizes. Panel
//! backgrounds (translucent black boxes) are found by the strongest
//! darkening step near each edge of a seed box. Writes a report and
//! enlarged crops (reference | ours | overlay: reference red, ours green).

use std::{fmt::Write as _, path::Path};

use image::{Rgb, RgbImage, imageops};

/// Which screen edge a region's x counts from.
#[derive(Clone, Copy)]
pub enum Anchor {
    Left,
    Centre,
    Right,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// HUD-coloured ink: offset by correlation, ink box.
    Ink,
    /// A translucent panel background: edges.
    Box,
}

/// A region in virtual units (480 lines): x0..x1 from the anchor (for
/// `Right`, distances back from the right edge, x0 > x1), y0..y1 from the
/// top.
pub struct Region {
    pub name: &'static str,
    /// Only in captures of this state (`ak`, `select`); None for all.
    pub state: Option<&'static str>,
    pub kind: Kind,
    pub anchor: Anchor,
    pub x: [f32; 2],
    pub y: [f32; 2],
}

const fn r(
    name: &'static str,
    state: Option<&'static str>,
    kind: Kind,
    anchor: Anchor,
    x: [f32; 2],
    y: [f32; 2],
) -> Region {
    Region {
        name,
        state,
        kind,
        anchor,
        x,
        y,
    }
}

use Anchor::{Centre, Left, Right};
use Kind::{Box, Ink};

/// The regions. Ink regions are generous (the search moves within them);
/// box regions are the panel's expected box.
pub const REGIONS: &[Region] = &[
    r("health_icon", None, Ink, Left, [10.0, 40.0], [442.0, 474.0]),
    r("health_digits", None, Ink, Left, [40.0, 92.0], [442.0, 474.0]),
    r("health_box", None, Box, Left, [8.0, 88.0], [446.0, 471.0]),
    r("armor_icon", None, Ink, Left, [150.0, 180.0], [442.0, 474.0]),
    r("armor_digits", None, Ink, Left, [180.0, 232.0], [442.0, 474.0]),
    r("armor_box", None, Box, Left, [148.0, 228.0], [446.0, 471.0]),
    r("timer_icon", None, Ink, Centre, [-30.0, 0.0], [442.0, 474.0]),
    r("timer_digits", None, Ink, Centre, [4.0, 70.0], [442.0, 474.0]),
    r("timer_box", None, Box, Centre, [-28.0, 68.0], [446.0, 471.0]),
    r("money_icon", None, Ink, Right, [120.0, 96.0], [410.0, 440.0]),
    r("money_digits", None, Ink, Right, [80.0, 12.0], [410.0, 440.0]),
    r("ammo_digits", None, Ink, Right, [145.0, 48.0], [442.0, 474.0]),
    r("ammo_icon", None, Ink, Right, [44.0, 10.0], [442.0, 474.0]),
    r("ammo_box", None, Box, Right, [140.0, 8.0], [446.0, 471.0]),
    // Weapon selection, slot 1 drawn large (`ak`) or slot 2 (`select`).
    r("sel_numbers", None, Ink, Centre, [-200.0, 200.0], [18.0, 30.0]),
    r("sel_icon", Some("ak"), Ink, Centre, [-182.0, -80.0], [28.0, 78.0]),
    r("sel_name", Some("ak"), Ink, Centre, [-190.0, -80.0], [80.0, 96.0]),
    r("sel_box", Some("ak"), Box, Centre, [-190.0, -82.0], [16.0, 96.0]),
    r("sel_icon", Some("select"), Ink, Centre, [-114.0, -10.0], [28.0, 78.0]),
    r("sel_name", Some("select"), Ink, Centre, [-124.0, -10.0], [80.0, 96.0]),
    r("sel_box", Some("select"), Box, Centre, [-122.0, -14.0], [16.0, 96.0]),
    // The pickup history's newest row (its other rows show over the
    // crates in the reference).
    r("history", Some("select"), Ink, Right, [100.0, 14.0], [290.0, 345.0]),
    // The radar's box (top left).
    r("radar_box", None, Box, Left, [16.0, 110.0], [16.0, 110.0]),
];

/// The region's pixel rectangle [x0, y0, x1, y1) in a `w` x `h` image.
pub fn pixels(region: &Region, w: u32, h: u32) -> [i32; 4] {
    let s = h as f32 / 480.0;
    let x = |u: f32| match region.anchor {
        Left => u * s,
        Centre => w as f32 / 2.0 + u * s,
        Right => w as f32 - u * s,
    };
    let (xa, xb) = (x(region.x[0]), x(region.x[1]));
    [
        xa.min(xb).round() as i32,
        (region.y[0] * s).round() as i32,
        xa.max(xb).round() as i32,
        (region.y[1] * s).round() as i32,
    ]
}

/// How much a pixel looks like HUD ink: orange to yellow, saturated,
/// bright enough (the HUD colour over any background), 0..1.
fn ink(p: &Rgb<u8>) -> f32 {
    let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
    if r < 110.0 || g < 0.35 * r || g > 1.05 * r {
        return 0.0;
    }
    ((r - b - 45.0) / 80.0).clamp(0.0, 1.0)
}

fn luma(p: &Rgb<u8>) -> f32 {
    0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32
}

/// The weight map of `rect` (padded by `pad` on every side; outside the
/// image is 0), row-major.
fn weights(img: &RgbImage, rect: [i32; 4], pad: i32) -> (Vec<f32>, i32, i32) {
    let (w, h) = (rect[2] - rect[0] + 2 * pad, rect[3] - rect[1] + 2 * pad);
    let mut out = vec![0.0; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let (ix, iy) = (rect[0] - pad + x, rect[1] - pad + y);
            if ix >= 0 && iy >= 0 && (ix as u32) < img.width() && (iy as u32) < img.height() {
                out[(y * w + x) as usize] = ink(img.get_pixel(ix as u32, iy as u32));
            }
        }
    }
    (out, w, h)
}

/// The ink box of a weight map (pixels over half weight), relative to the
/// map; None when (almost) empty.
fn ink_box(map: &[f32], w: i32, h: i32) -> Option<[i32; 4]> {
    let mut b = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
    let mut n = 0;
    for y in 0..h {
        for x in 0..w {
            if map[(y * w + x) as usize] > 0.5 {
                b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
                n += 1;
            }
        }
    }
    (n >= 4).then_some(b)
}

/// One measured part.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Measure {
    pub region: String,
    /// Ours minus the reference, pixels (x right, y down).
    pub offset: Option<[i32; 2]>,
    /// Ink or box rectangles [x0, y0, x1, y1] in screen pixels.
    pub reference: Option<[i32; 4]>,
    pub ours: Option<[i32; 4]>,
    /// Normalized correlation at the best offset (ink only).
    pub score: f32,
}

/// The best shift of `ours` onto `reference` (both padded maps of the same
/// region): maximises the normalized cross-correlation.
fn best_shift(a: &[f32], b: &[f32], w: i32, h: i32, pad: i32) -> ([i32; 2], f32) {
    let norm = |m: &[f32]| m.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
    let (na, nb) = (norm(a), norm(b));
    let mut best = ([0, 0], f32::MIN);
    for dy in -pad..=pad {
        for dx in -pad..=pad {
            let mut s = 0.0;
            for y in pad..h - pad {
                for x in pad..w - pad {
                    let (ox, oy) = (x + dx, y + dy);
                    s += a[(y * w + x) as usize] * b[(oy * w + ox) as usize];
                }
            }
            // Prefer the smaller shift on ties.
            let s = s / (na * nb) - 1e-6 * (dx.abs() + dy.abs()) as f32;
            if s > best.1 {
                best = ([dx, dy], s);
            }
        }
    }
    best
}

fn measure_ink(name: &str, reference: &RgbImage, ours: &RgbImage, rect: [i32; 4]) -> Measure {
    let pad = (12.0 * reference.height() as f32 / 720.0).round() as i32;
    let (a, w, h) = weights(reference, rect, pad);
    let (b, _, _) = weights(ours, rect, pad);
    // Ink boxes inside the region proper (not the padding).
    let inner = |m: &[f32]| {
        let mut m = m.to_vec();
        for y in 0..h {
            for x in 0..w {
                if x < pad || y < pad || x >= w - pad || y >= h - pad {
                    m[(y * w + x) as usize] = 0.0;
                }
            }
        }
        ink_box(&m, w, h).map(|b| {
            [
                b[0] + rect[0] - pad,
                b[1] + rect[1] - pad,
                b[2] + rect[0] - pad,
                b[3] + rect[1] - pad,
            ]
        })
    };
    let (rb, ob) = (inner(&a), inner(&b));
    let (offset, score) = if rb.is_some() && ob.is_some() {
        let (o, s) = best_shift(&a, &b, w, h, pad);
        (Some(o), s)
    } else {
        (None, 0.0)
    };
    Measure {
        region: name.to_string(),
        offset,
        reference: rb,
        ours: ob,
        score,
    }
}

/// A translucent box's edges near `rect`: on each side the strongest step
/// from lighter outside to darker inside within `search` pixels, measured
/// on the middle half of the side.
fn box_edges(img: &RgbImage, rect: [i32; 4], search: i32) -> Option<[i32; 4]> {
    let at = |x: i32, y: i32| {
        if x < 0 || y < 0 || x as u32 >= img.width() || y as u32 >= img.height() {
            None
        } else {
            Some(luma(img.get_pixel(x as u32, y as u32)))
        }
    };
    let [x0, y0, x1, y1] = rect;
    let (my0, my1) = (y0 + (y1 - y0) / 4, y1 - (y1 - y0) / 4);
    let (mx0, mx1) = (x0 + (x1 - x0) / 4, x1 - (x1 - x0) / 4);
    let column = |x: i32| -> Option<f32> {
        let v: Vec<f32> = (my0..my1).filter_map(|y| at(x, y)).collect();
        (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32)
    };
    let row = |y: i32| -> Option<f32> {
        let v: Vec<f32> = (mx0..mx1).filter_map(|x| at(x, y)).collect();
        (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32)
    };
    // The first pixel inside: the step from `outside` (sign -1: left or
    // top, the outside is before) to inside.
    let edge = |line: &dyn Fn(i32) -> Option<f32>, centre: i32, before_is_outside: bool| -> Option<i32> {
        let mut best: Option<(i32, f32)> = None;
        for p in centre - search..=centre + search {
            let (a, b) = (line(p - 1)?, line(p)?);
            // Darker inside: outside minus inside.
            let step = if before_is_outside { a - b } else { b - a };
            if best.is_none_or(|(_, s)| step > s) {
                best = Some((p, step));
            }
        }
        let (p, s) = best?;
        (s > 3.0).then_some(p)
    };
    Some([
        edge(&column, x0, true)?,
        edge(&row, y0, true)?,
        edge(&column, x1, false)?,
        edge(&row, y1, false)?,
    ])
}

fn measure_box(name: &str, reference: &RgbImage, ours: &RgbImage, rect: [i32; 4]) -> Measure {
    let search = (10.0 * reference.height() as f32 / 720.0).round() as i32;
    let (rb, ob) = (box_edges(reference, rect, search), box_edges(ours, rect, search));
    let offset = match (rb, ob) {
        (Some(a), Some(b)) => Some([b[0] - a[0], b[1] - a[1]]),
        _ => None,
    };
    Measure {
        region: name.to_string(),
        offset,
        reference: rb,
        ours: ob,
        score: 0.0,
    }
}

/// An enlarged crop: reference | ours | overlay (reference ink red, ours
/// green).
fn sheet(reference: &RgbImage, ours: &RgbImage, rect: [i32; 4], zoom: u32) -> RgbImage {
    let pad = 6;
    let (x, y) = ((rect[0] - pad).max(0) as u32, (rect[1] - pad).max(0) as u32);
    let w = ((rect[2] - rect[0] + 2 * pad) as u32).min(reference.width() - x);
    let h = ((rect[3] - rect[1] + 2 * pad) as u32).min(reference.height() - y);
    let a = imageops::crop_imm(reference, x, y, w, h).to_image();
    let b = imageops::crop_imm(ours, x, y, w, h).to_image();
    let mut overlay = RgbImage::new(w, h);
    for (o, (pa, pb)) in overlay.pixels_mut().zip(a.pixels().zip(b.pixels())) {
        *o = Rgb([(ink(pa) * 255.0) as u8, (ink(pb) * 255.0) as u8, 0]);
    }
    let mut out = RgbImage::new(w * 3 * zoom + 2 * zoom, h * zoom);
    for (i, img) in [a, b, overlay].iter().enumerate() {
        let big = imageops::resize(img, w * zoom, h * zoom, imageops::FilterType::Nearest);
        imageops::replace(&mut out, &big, (i as u32 * (w * zoom + zoom)) as i64, 0);
    }
    out
}

/// Compare every `hudref_<state>_<W>x<H>.jpg` in `ref_dir` with
/// `ours_<state>_<W>x<H>.png` in `ours_dir`; report and crops in `out`.
pub fn run(ref_dir: &Path, ours_dir: &Path, out: &Path) -> Result<Vec<(String, Vec<Measure>)>, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut names: Vec<String> = std::fs::read_dir(ref_dir)
        .map_err(|e| format!("{}: {e}", ref_dir.display()))?
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.starts_with("hudref_") && n.ends_with(".jpg"))
        .collect();
    names.sort();
    let mut text = String::new();
    let mut all = Vec::new();
    for name in names {
        let stem = name.trim_start_matches("hudref_").trim_end_matches(".jpg").to_string();
        let state = stem.split('_').next().unwrap_or_default().to_string();
        let ours_path = ours_dir.join(format!("ours_{stem}.png"));
        let (Ok(reference), Ok(ours)) = (image::open(ref_dir.join(&name)), image::open(&ours_path)) else {
            let _ = writeln!(text, "{stem}: missing {}", ours_path.display());
            continue;
        };
        let (reference, ours) = (reference.to_rgb8(), ours.to_rgb8());
        if reference.dimensions() != ours.dimensions() {
            let _ = writeln!(
                text,
                "{stem}: sizes differ ({:?} vs {:?})",
                reference.dimensions(),
                ours.dimensions()
            );
            continue;
        }
        let (w, h) = reference.dimensions();
        let _ = writeln!(
            text,
            "\n{stem}  (offset = ours - reference, px; rects x0,y0,x1,y1)\n{:<14} {:>9} {:>22} {:>22} {:>6}",
            "region", "offset", "reference", "ours", "score"
        );
        let mut rows = Vec::new();
        for region in REGIONS.iter().filter(|r| r.state.is_none_or(|s| s == state)) {
            let rect = pixels(region, w, h);
            let m = match region.kind {
                Ink => measure_ink(region.name, &reference, &ours, rect),
                Box => measure_box(region.name, &reference, &ours, rect),
            };
            let f = |r: Option<[i32; 4]>| {
                r.map_or("-".to_string(), |r| {
                    format!("{},{},{},{} ({}x{})", r[0], r[1], r[2], r[3], r[2] - r[0], r[3] - r[1])
                })
            };
            let _ = writeln!(
                text,
                "{:<14} {:>9} {:>22} {:>22} {:>6.2}",
                m.region,
                m.offset.map_or("-".to_string(), |o| format!("{:+},{:+}", o[0], o[1])),
                f(m.reference),
                f(m.ours),
                m.score
            );
            let zoom = if h >= 1000 { 2 } else { 3 };
            let crop = sheet(&reference, &ours, rect, zoom);
            let path = out.join(format!("{stem}_{}.png", region.name));
            crop.save(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            rows.push(m);
        }
        all.push((stem, rows));
    }
    print!("{text}");
    std::fs::write(out.join("hudcmp.txt"), &text).map_err(|e| e.to_string())?;
    let json: Vec<_> = all
        .iter()
        .map(|(s, m)| serde_json::json!({"capture": s, "parts": m}))
        .collect();
    std::fs::write(out.join("hudcmp.json"), serde_json::to_string_pretty(&json).unwrap()).map_err(|e| e.to_string())?;
    println!(
        "\ncrops (reference | ours | overlay: reference red, ours green) in {}",
        out.display()
    );
    Ok(all)
}
