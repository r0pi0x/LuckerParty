//! CS:S's breakable brushes (specs/source/breakables.md): the gib lists
//! from the install's `scripts/propdata.txt` ("BreakableModels": WoodChunks,
//! GlassChunks, MetalChunks, ConcreteChunks, plus any list or model a
//! map's `gibmodel` names) and how gibs fly. The breaking itself is the
//! logic layer's (`logic::breakables`).

use super::material::MaterialLoader;
use crate::map::{
    MapEntity,
    breakables::{MapGibPhysics, MapGibSet},
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

/// Load the gib lists the map's breakables can use.
pub fn load_gibs(materials: &mut MaterialLoader, entities: &[MapEntity], warnings: &mut Vec<String>) -> Vec<MapGibSet> {
    if !entities.iter().any(|e| CLASSES.contains(&e.classname())) {
        return Vec::new();
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
    let mut out = Vec::new();
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
