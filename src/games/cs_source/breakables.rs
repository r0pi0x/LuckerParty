//! CS:S's breakable brushes (specs/source/breakables.md): the gib lists
//! from the install's `scripts/propdata.txt` ("BreakableModels": WoodChunks,
//! GlassChunks, MetalChunks, ConcreteChunks, plus any list or model a
//! map's `gibmodel` names) and how gibs fly. The breaking itself is the
//! logic layer's (`logic::breakables`).

use super::material::MaterialLoader;
use crate::map::{
    MapEntity, MapMesh, MapModel, PaneLook,
    breakables::{MapGibPhysics, MapGibSet, PANE_PIECES},
};

const UNIT: f32 = 0.0254;
/// sv_gravity (units/s²); gibs fall at half of it.
const GRAVITY: f32 = 800.0;
/// Glass gibs: alpha 128 of 255, bounce 0.3.
const GLASS_ALPHA: f32 = 128.0 / 255.0;
const GLASS_BOUNCE: f32 = 0.3;
/// Other gibs' bounce (not in the spec: a guess).
const BOUNCE: f32 = 0.5;

/// Brush entity classes that break (logic `Breakable`): drawn and solid
/// through their own node so they can disappear.
pub const CLASSES: &[&str] = &["func_breakable", "func_breakable_surf"];

/// The gib lists every breakable material uses.
const LISTS: &[&str] = &["WoodChunks", "GlassChunks", "MetalChunks", "ConcreteChunks"];

pub fn gib_physics() -> MapGibPhysics {
    MapGibPhysics {
        gravity: GRAVITY / 2.0 * UNIT,
        bounce: BOUNCE,
        glass_bounce: GLASS_BOUNCE,
        glass_alpha: GLASS_ALPHA,
        fade: 1.0,
        // Broken props' pieces are client physics props: full gravity.
        prop_gravity: GRAVITY * UNIT,
        // The shared temporary-entity bounce sound rule (view_models.md
        // 8, which breakables.md "Breaking" 5 points to by its break
        // flag): one bounce in six, full volume at 450 units/s down, the
        // entry's pitch range one time in four.
        sound_chance: 1.0 / 6.0,
        sound_full_speed: 450.0 * UNIT,
        sound_random_pitch: 0.25,
    }
}

/// The model paths of each "BreakableModels" list in propdata.txt.
pub fn breakable_models(text: &str) -> Vec<(String, Vec<String>)> {
    let kv = super::hud::parse(text);
    // The file is one block ("PropData.txt" { ... }) holding the lists.
    let root = kv.items().first().map(|(_, v)| v);
    let Some(lists) = root
        .and_then(|r| r.get("BreakableModels"))
        .or_else(|| kv.get("BreakableModels"))
    else {
        return Vec::new();
    };
    lists
        .items()
        .iter()
        .map(|(name, models)| {
            (
                name.clone(),
                models.items().iter().map(|(path, _)| path.clone()).collect(),
            )
        })
        .collect()
}

/// The falling piece a collapsing window pane drops (specs/source/
/// breakables.md "Support pass"), one model per body (0..2).
pub const PANE_PIECE_MODEL: &str = "models/brokenglass_piece.mdl";

/// The jagged edge materials a broken glass window draws where unbroken
/// panes meet broken ones (the spec names "glassbroken_*"; these are the
/// install's edge textures, a shard rising from the texture's bottom).
pub const GLASS_EDGES: [&str; 12] = [
    "models/brokenglass/glassbroken_01a",
    "models/brokenglass/glassbroken_01b",
    "models/brokenglass/glassbroken_01c",
    "models/brokenglass/glassbroken_01d",
    "models/brokenglass/glassbroken_02a",
    "models/brokenglass/glassbroken_02b",
    "models/brokenglass/glassbroken_02c",
    "models/brokenglass/glassbroken_02d",
    "models/brokenglass/glassbroken_03a",
    "models/brokenglass/glassbroken_03b",
    "models/brokenglass/glassbroken_03c",
    "models/brokenglass/glassbroken_03d",
];

/// Each window's broken looks (`MapMesh::pane_look`): its face again in
/// the face material's `$crackmaterial`, and, for glass, the face as a
/// template for each edge material. Both are double sided (the spec
/// leaves which side the client draws open).
pub fn add_window_looks(materials: &mut MaterialLoader, entities: &[MapEntity], meshes: &mut Vec<MapMesh>) {
    let windows: Vec<(usize, bool)> = entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.classname() == "func_breakable_surf")
        .map(|(i, e)| (i, e.get("surfacetype") == Some("1")))
        .collect();
    if windows.is_empty() {
        return;
    }
    let mut extra = Vec::new();
    for m in meshes.iter() {
        let Some(&(_, tile)) = m.entity.and_then(|i| windows.iter().find(|(w, _)| *w == i)) else {
            continue;
        };
        if m.pane_look != PaneLook::Whole || m.texture.is_none() {
            continue;
        }
        if let Some(crack) = materials.material_value(&m.material, "$crackmaterial") {
            let crack = crack.replace('\\', "/");
            let r = materials.resolve(&crack);
            if r.texture.is_some() {
                extra.push(MapMesh {
                    material: crack,
                    texture: r.texture,
                    normal_map: r.normal_map,
                    detail: r.detail,
                    alpha: r.alpha,
                    double_sided: true,
                    envmap: r.envmap,
                    tint: r.tint,
                    unlit: r.unlit,
                    pane_look: PaneLook::Cracked,
                    ..m.clone()
                });
            }
        }
        if tile {
            continue;
        }
        for (k, name) in GLASS_EDGES.iter().enumerate() {
            let r = materials.resolve(name);
            if r.texture.is_none() {
                continue;
            }
            extra.push(MapMesh {
                material: name.to_string(),
                texture: r.texture,
                normal_map: None,
                detail: None,
                blend: None,
                alpha: r.alpha,
                double_sided: true,
                envmap: None,
                tint: None,
                unlit: r.unlit,
                // Drawn by its own texture's light (UnlitGeneric).
                lightmap_uvs: Vec::new(),
                pane_look: PaneLook::Edge(k as u8),
                ..m.clone()
            });
        }
    }
    meshes.extend(extra);
}

/// The falling pane piece's models, one per body.
fn pane_pieces(materials: &mut MaterialLoader) -> Result<Vec<MapModel>, String> {
    let model = super::props::load_shell(materials, PANE_PIECE_MODEL)?;
    Ok((0..crate::map::breakables::PANE_PIECE_BODIES as i32)
        .map(|body| {
            let mut m = model.clone();
            m.meshes
                .retain(|mesh| mesh.body.is_none_or(|(part, choice)| model.body_choice(body, part as usize) == choice));
            m.body_parts.clear();
            m
        })
        .collect())
}

/// Load the gib lists the map's breakables can use.
pub fn load_gibs(materials: &mut MaterialLoader, entities: &[MapEntity], warnings: &mut Vec<String>) -> Vec<MapGibSet> {
    if !entities.iter().any(|e| CLASSES.contains(&e.classname())) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if entities.iter().any(|e| e.classname() == "func_breakable_surf") {
        match pane_pieces(materials) {
            Ok(models) => out.push(MapGibSet {
                name: PANE_PIECES.to_string(),
                models,
            }),
            Err(e) => warnings.push(e),
        }
    }
    let lists = materials
        .read("scripts/propdata.txt")
        .map(|b| breakable_models(&String::from_utf8_lossy(&b)))
        .unwrap_or_default();
    let mut wanted: Vec<String> = LISTS.iter().map(|s| s.to_string()).collect();
    for e in entities.iter().filter(|e| e.classname() == "func_breakable") {
        if let Some(g) = e.get("gibmodel").filter(|g| !g.is_empty())
            && !wanted.iter().any(|w| w.eq_ignore_ascii_case(g))
        {
            wanted.push(g.to_string());
        }
    }
    for name in wanted {
        // A list name, or a single model.
        let paths: Vec<String> = match lists.iter().find(|(n, _)| n.eq_ignore_ascii_case(&name)) {
            Some((_, paths)) => paths.clone(),
            None if name.to_ascii_lowercase().ends_with(".mdl") => vec![name.clone()],
            None => {
                warnings.push(format!("propdata: no gib list {name}"));
                continue;
            }
        };
        let models = paths
            .iter()
            .filter_map(|p| match super::props::load_shell(materials, p) {
                Ok(m) => Some(m),
                Err(e) => {
                    warnings.push(e);
                    None
                }
            })
            .collect();
        out.push(MapGibSet { name, models });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_breakable_model_lists() {
        let text = r#"
"PropData.txt"
{
    "sections" { "Wooden.Tiny" { "health" "1" } }
    "BreakableModels"
    {
        "MetalChunks"
        {
            "models/gibs/metal_gib1.mdl"  "1"
            "models/gibs/metal_gib2.mdl"  "1"
        }
        "GlassChunks" { "models/gibs/glass_shard01.mdl" "1" }
    }
}
"#;
        let lists = breakable_models(text);
        assert_eq!(lists.len(), 2);
        assert_eq!(lists[0].0, "MetalChunks");
        assert_eq!(
            lists[0].1,
            vec!["models/gibs/metal_gib1.mdl", "models/gibs/metal_gib2.mdl"]
        );
        assert_eq!(lists[1].1.len(), 1);
    }
}
