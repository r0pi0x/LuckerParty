//! A prop model's prop data (specs/source/prop_damage.md 2.1): the
//! model's `prop_data` block over the `scripts/propdata.txt` template it
//! names (recursively), its fire and physgun interactions, and the pieces
//! its `.phy` says it breaks into (7.3).

use bevy::prelude::*;

use super::hud::{self, Kv};

/// A model's resolved prop data.
#[derive(Clone, Debug, PartialEq)]
pub struct PropData {
    pub health: Option<i32>,
    /// Damage multipliers: bullets, club, blast.
    pub bullets: f32,
    pub club: f32,
    pub explosive: f32,
    pub physicsmode: Option<i32>,
    /// A gib list name (propdata "BreakableModels") and how many chunks.
    pub breakable_model: Option<String>,
    pub breakable_count: i32,
    pub breakable_skin: i32,
    pub damage_table: Option<String>,
    pub explosive_damage: Option<f32>,
    pub explosive_radius: Option<f32>,
    pub allowstatic: bool,
    /// Interactions that matter (spec 2.1 step 4): `flammable`,
    /// `explosive_resist`, `ignite_halfhealth`, `explode_fire`,
    /// `firstimpact_break`.
    pub interactions: Vec<&'static str>,
    /// A template it names doesn't exist.
    pub missing_template: bool,
}

impl Default for PropData {
    fn default() -> Self {
        Self {
            health: None,
            bullets: 1.0,
            club: 1.0,
            explosive: 1.0,
            physicsmode: None,
            breakable_model: None,
            breakable_count: 0,
            breakable_skin: 0,
            damage_table: None,
            explosive_damage: None,
            explosive_radius: None,
            allowstatic: false,
            interactions: Vec::new(),
            missing_template: false,
        }
    }
}

/// The first block named `name` anywhere under `kv` (depth first).
fn find<'a>(kv: &'a Kv, name: &str) -> Option<&'a Kv> {
    for (k, v) in kv.items() {
        if let Kv::Block(_) = v {
            if k.eq_ignore_ascii_case(name) {
                return Some(v);
            }
            if let Some(f) = find(v, name) {
                return Some(f);
            }
        }
    }
    None
}

fn interactions(block: &Kv, kind: &str, out: &mut Vec<&'static str>) {
    for (k, v) in block.items() {
        let Kv::Value(v) = v else { continue };
        let (k, v) = (k.to_ascii_lowercase(), v.trim().to_ascii_lowercase());
        let found = match (kind, k.as_str(), v.as_str()) {
            ("fire", "flammable", "yes") => "flammable",
            ("fire", "explosive_resist", "yes") => "explosive_resist",
            ("fire", "ignite", "halfhealth") => "ignite_halfhealth",
            ("physgun", "onbreak", "explode_fire") => "explode_fire",
            ("physgun", "onfirstimpact", "break") => "firstimpact_break",
            _ => continue,
        };
        if !out.contains(&found) {
            out.push(found);
        }
    }
}

/// Apply a prop data block (a template or the model's own): its base
/// first, then its own keys.
fn apply(block: &Kv, templates: &Kv, pd: &mut PropData, depth: u32) {
    if let Some(base) = block.str("base").map(str::trim).filter(|b| !b.is_empty()) {
        match find(templates, base) {
            Some(t) if depth < 16 => apply(t, templates, pd, depth + 1),
            Some(_) => {}
            None => pd.missing_template = true,
        }
    }
    let f = |v: &str| v.trim().parse::<f32>().ok();
    let i = |v: &str| v.trim().parse::<f32>().ok().map(|x| x as i32);
    for (k, v) in block.items() {
        match v {
            Kv::Block(_) => match k.to_ascii_lowercase().as_str() {
                "fire_interactions" => interactions(v, "fire", &mut pd.interactions),
                "physgun_interactions" => interactions(v, "physgun", &mut pd.interactions),
                _ => {}
            },
            Kv::Value(v) => match k.to_ascii_lowercase().as_str() {
                "health" => pd.health = i(v).or(pd.health),
                "dmg.bullets" => pd.bullets = f(v).unwrap_or(pd.bullets),
                "dmg.club" => pd.club = f(v).unwrap_or(pd.club),
                "dmg.explosive" => pd.explosive = f(v).unwrap_or(pd.explosive),
                "physicsmode" => pd.physicsmode = i(v).or(pd.physicsmode),
                "breakable_model" => pd.breakable_model = Some(v.trim().to_string()).filter(|s| !s.is_empty()),
                "breakable_count" => pd.breakable_count = i(v).unwrap_or(pd.breakable_count),
                "breakable_skin" => pd.breakable_skin = i(v).unwrap_or(pd.breakable_skin),
                "damage_table" => pd.damage_table = Some(v.trim().to_string()).filter(|s| !s.is_empty()),
                "explosive_damage" => pd.explosive_damage = f(v).or(pd.explosive_damage),
                "explosive_radius" => pd.explosive_radius = f(v).or(pd.explosive_radius),
                "allowstatic" => pd.allowstatic = i(v).is_some_and(|x| x != 0),
                _ => {}
            },
        }
    }
}

/// propdata.txt's templates.
pub(crate) fn templates(text: &str) -> Kv {
    hud::parse(text)
}

/// A model's prop data from its key values text, over `templates`
/// (propdata.txt); None when the model has no `prop_data` block.
pub(crate) fn resolve(model_kv: &str, templates: &Kv) -> Option<PropData> {
    let kv = hud::parse(model_kv);
    let own = find(&kv, "prop_data")?;
    let mut pd = PropData::default();
    apply(own, templates, &mut pd, 0);
    // Interaction blocks beside prop_data.
    for (k, v) in kv.items().iter().chain(kv.items().iter().flat_map(|(_, v)| v.items())) {
        match k.to_ascii_lowercase().as_str() {
            "fire_interactions" => interactions(v, "fire", &mut pd.interactions),
            "physgun_interactions" => interactions(v, "physgun", &mut pd.interactions),
            _ => {}
        }
    }
    Some(pd)
}

/// A gib list for a chunk name: the first list whose name starts with it
/// (spec 2.1).
pub fn chunk_list<'a>(lists: &'a [(String, Vec<String>)], name: &str) -> Option<&'a (String, Vec<String>)> {
    let name = name.to_ascii_lowercase();
    lists.iter().find(|(n, _)| n.to_ascii_lowercase().starts_with(&name))
}

/// The template chunk size limit for a prop box `size` (units, spec 2.1
/// step 5): the smallest axis replaced by 1 (ties pick the later axis),
/// the product over 1024.
pub fn chunk_size_limit(size: Vec3) -> usize {
    let (x, y, z) = (size.x, size.y, size.z);
    let smallest = if x < y && x < z {
        0
    } else if x < y {
        2
    } else if y < z {
        1
    } else {
        2
    };
    let mut s = size;
    s[smallest] = 1.0;
    ((s.x * s.y * s.z) / 1024.0).floor().max(0.0) as usize
}

/// One `.phy` break block (spec 7.3).
#[derive(Clone, Debug, PartialEq)]
pub struct BreakPiece {
    /// Model path, `models/...mdl`.
    pub model: String,
    /// Offset in the prop's model space, units.
    pub offset: Vec3,
    /// Seconds before it fades; 0 never.
    pub fadetime: f32,
    /// Outward speed, units/s.
    pub burst: f32,
    pub motion_disabled: bool,
}

/// A `.phy`'s break blocks as pieces.
pub fn break_pieces(blocks: &[Vec<(String, String)>]) -> Vec<BreakPiece> {
    blocks
        .iter()
        .filter_map(|keys| {
            let get = |k: &str| keys.iter().find(|(key, _)| key == k).map(|(_, v)| v.trim());
            let mut model = get("model")?.replace('\\', "/").to_ascii_lowercase();
            if model.is_empty() {
                return None;
            }
            if !model.starts_with("models/") {
                model = format!("models/{model}");
            }
            if !model.ends_with(".mdl") {
                model.push_str(".mdl");
            }
            let num = |k: &str, d: f32| get(k).and_then(|v| v.parse::<f32>().ok()).unwrap_or(d);
            Some(BreakPiece {
                model,
                offset: get("offset").map_or(Vec3::ZERO, crate::map::entities::parse_vector),
                fadetime: num("fadetime", 20.0),
                burst: num("burst", 100.0),
                motion_disabled: num("motiondisabled", 0.0) != 0.0,
            })
        })
        .collect()
}

/// The break sound entry of a surface property (its `break` key through
/// `base`; spec 7.4).
pub fn break_sound(surfaces: &super::surfaceprops::SurfaceProps, surface: &str) -> Option<String> {
    let s = surface.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("default") {
        return None;
    }
    surfaces.text(s, "break").filter(|b| !b.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATES: &str = r#"
"PropData.txt"
{
    "sections"
    {
        "Wooden"
        {
            "Wooden.Base" { "dmg.bullets" "0.75" "dmg.club" "2.0" "dmg.explosive" "1.5" "breakable_model" "WoodChunks" "breakable_skin" "0" }
            "Wooden.Medium" { "base" "Wooden.Base" "health" "30" "breakable_count" "4" }
            "Wooden.Large" { "base" "Wooden.Base" "health" "50" "physicsmode" "1" "breakable_count" "6" }
        }
        "Metal.Base" { "health" "0" }
        "Metal.Medium" { "base" "Metal.Base" "physicsmode" "1" }
    }
    "BreakableModels"
    {
        "WoodChunks" { "models/gibs/wood_gib01e.mdl" "1" "models/gibs/wood_gib01d.mdl" "1" }
    }
}
"#;

    #[test]
    fn templates_resolve_recursively_and_models_override() {
        let t = templates(TEMPLATES);
        let pd = resolve(r#""prop_data" { "base" "Wooden.Medium" }"#, &t).unwrap();
        assert_eq!(pd.health, Some(30));
        assert_eq!((pd.bullets, pd.club, pd.explosive), (0.75, 2.0, 1.5));
        assert_eq!(pd.breakable_model.as_deref(), Some("WoodChunks"));
        assert_eq!(pd.breakable_count, 4);
        let pd = resolve(r#""prop_data" { "base" "Wooden.Large" "health" "20" }"#, &t).unwrap();
        assert_eq!((pd.health, pd.physicsmode), (Some(20), Some(1)));
        let pd = resolve(r#""prop_data" { "base" "Metal.Medium" }"#, &t).unwrap();
        assert_eq!(pd.health, Some(0));
        assert!(!pd.missing_template);
        let pd = resolve(r#""prop_data" { "base" "Nope" }"#, &t).unwrap();
        assert!(pd.missing_template);
        assert!(resolve(r#""physgun_interactions" { }"#, &t).is_none());
    }

    #[test]
    fn interactions_beside_prop_data() {
        let t = templates(TEMPLATES);
        let text = r#""prop_data" { "base" "Metal.Medium" "health" "20" "explosive_damage" "25" "explosive_radius" "80" }
            "physgun_interactions" { "onbreak" "explode_fire" }
            "fire_interactions" { "flammable" "yes" "ignite" "halfhealth" }"#;
        let pd = resolve(text, &t).unwrap();
        assert_eq!(pd.explosive_damage, Some(25.0));
        assert_eq!(pd.explosive_radius, Some(80.0));
        assert_eq!(pd.interactions, vec!["explode_fire", "flammable", "ignite_halfhealth"]);
    }

    #[test]
    fn chunk_size_limit_and_lists() {
        // woodbarrel001 (spec test case): 28.9 x 28.9 x 40.6 -> 1.
        assert_eq!(chunk_size_limit(Vec3::new(28.9, 28.9, 40.6)), 1);
        assert_eq!(chunk_size_limit(Vec3::new(40.5, 40.3, 40.2)), 1);
        assert_eq!(chunk_size_limit(Vec3::new(100.0, 100.0, 10.0)), 9);
        let lists = vec![("WoodChunks".to_string(), vec!["a".to_string()])];
        assert!(chunk_list(&lists, "wood").is_some());
        assert!(chunk_list(&lists, "Metal").is_none());
    }

    #[test]
    fn break_blocks_become_pieces() {
        let blocks = vec![
            vec![
                ("model".into(), "props_junk/wood_crate001a_chunk01".into()),
                ("fadetime".into(), "10".into()),
            ],
            vec![
                ("model".into(), "models/a.mdl".into()),
                ("motiondisabled".into(), "1".into()),
            ],
        ];
        let p = break_pieces(&blocks);
        assert_eq!(p[0].model, "models/props_junk/wood_crate001a_chunk01.mdl");
        assert_eq!((p[0].fadetime, p[0].burst), (10.0, 100.0));
        assert!(p[1].motion_disabled && p[1].fadetime == 20.0);
    }
}
