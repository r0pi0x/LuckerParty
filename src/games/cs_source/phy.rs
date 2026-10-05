//! Source collision models (`.phy`, next to a `.mdl`), from
//! specs/cs_source/physics_props.md section 1: a file header, one section
//! per solid holding a compact surface (a tree whose leaves are convex
//! pieces sharing one point array), then a key-values text section with the
//! physics parameters.

use bevy::prelude::*;

/// One solid's convex pieces and the parameters from its `solid` block.
#[derive(Clone, Debug)]
pub struct Phy {
    /// Convex pieces as triangles (Source model space, inches, Z up),
    /// wound counter-clockwise seen from outside.
    pub pieces: Vec<Vec<[Vec3; 3]>>,
    /// kg (default 1).
    pub mass: f32,
    pub surfaceprop: String,
    /// Linear and angular damping (defaults 0.1 when the key is missing).
    pub damping: f32,
    pub rotdamping: f32,
    /// Multiplier on the inertia tensor.
    pub inertia: f32,
}

const PHY_HEADER: usize = 16;
const VPHY_HEADER: usize = 28;
const NODE_SIZE: usize = 28;
const PIECE_HEADER: usize = 16;
const TRIANGLE_SIZE: usize = 16;
const POINT_SIZE: usize = 16;

fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn f32_at(b: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn offset(base: usize, rel: i32) -> Option<usize> {
    usize::try_from(base as i64 + rel as i64).ok()
}

/// IVP space (metres, Y down) to Source (inches, Z up).
fn to_source(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, z, -y) / 0.0254
}

/// Parse a `.phy`: the first `solid` block's solid (props use one).
pub fn parse(bytes: &[u8]) -> Result<Phy, String> {
    let header = i32_at(bytes, 0).ok_or("truncated header")? as usize;
    let count = i32_at(bytes, 8).ok_or("truncated header")?.max(0) as usize;
    if header != PHY_HEADER {
        return Err(format!("unexpected header size {header}"));
    }
    // Solid sections, then the text.
    let mut sections = Vec::with_capacity(count);
    let mut at = header;
    for _ in 0..count {
        let size = i32_at(bytes, at).ok_or("truncated solid")?.max(0) as usize;
        sections.push(at);
        at += 4 + size;
    }
    let text = bytes.get(at..).unwrap_or(&[]);
    let text = String::from_utf8_lossy(&text[..text.iter().position(|&c| c == 0).unwrap_or(text.len())]);
    let params = first_solid_block(&text);
    let index = params.get("index").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let section = *sections.get(index).ok_or("solid index out of range")?;
    let pieces = solid_pieces(bytes, section).ok_or("malformed solid")?;
    let num = |key: &str, default: f32| params.get(key).and_then(|v| v.parse().ok()).unwrap_or(default);
    Ok(Phy {
        pieces,
        mass: num("mass", 1.0),
        surfaceprop: params.get("surfaceprop").cloned().unwrap_or_default(),
        damping: num("damping", 0.1),
        rotdamping: num("rotdamping", 0.1),
        inertia: num("inertia", 1.0),
    })
}

/// A solid section's convex pieces: the leaves of its piece tree.
fn solid_pieces(b: &[u8], section: usize) -> Option<Vec<Vec<[Vec3; 3]>>> {
    let surface = if b.get(section + 4..section + 8)? == b"VPHY" {
        section + 4 + VPHY_HEADER
    } else {
        section + 4
    };
    let root = offset(surface, i32_at(b, surface + 32)?)?;
    let mut pieces = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let right = i32_at(b, node)?;
        if right == 0 {
            let piece = offset(node, i32_at(b, node + 4)?)?;
            pieces.push(piece_triangles(b, piece)?);
        } else {
            stack.push(offset(node, right)?);
            stack.push(node + NODE_SIZE);
        }
        if stack.len() > 1 << 16 {
            return None;
        }
    }
    Some(pieces)
}

fn piece_triangles(b: &[u8], piece: usize) -> Option<Vec<[Vec3; 3]>> {
    let points = offset(piece, i32_at(b, piece)?)?;
    let count = i16::from_le_bytes(b.get(piece + 12..piece + 14)?.try_into().ok()?).max(0) as usize;
    let point = |i: u32| -> Option<Vec3> {
        let at = points + POINT_SIZE * i as usize;
        Some(to_source(f32_at(b, at)?, f32_at(b, at + 4)?, f32_at(b, at + 8)?))
    };
    (0..count)
        .map(|t| {
            let tri = piece + PIECE_HEADER + TRIANGLE_SIZE * t;
            let start = |e: usize| u32_at(b, tri + 4 + 4 * e).map(|w| w & 0xffff);
            Some([point(start(0)?)?, point(start(1)?)?, point(start(2)?)?])
        })
        .collect()
}

/// Keys of the first `solid { ... }` block in the text section.
fn first_solid_block(text: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    // The block name stands alone before its brace (`solid {`).
    let Some(start) = text
        .match_indices("solid")
        .map(|(i, _)| i)
        .find(|&i| (i == 0 || text[..i].ends_with(char::is_whitespace)) && text[i + 5..].trim_start().starts_with('{'))
    else {
        return out;
    };
    let rest = &text[start..];
    let Some(open) = rest.find('{') else { return out };
    let Some(close) = rest[open..].find('}') else {
        return out;
    };
    for line in rest[open + 1..open + close].lines() {
        let parts: Vec<&str> = line.split('"').collect();
        // `"key" "value"` splits into ["", key, " ", value, ""].
        if parts.len() >= 4 {
            out.insert(parts[1].to_ascii_lowercase(), parts[3].to_string());
        }
    }
    out
}
