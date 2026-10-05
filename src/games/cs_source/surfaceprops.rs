//! Surface properties (specs/cs_source/physics_props.md 3.4): the game's
//! `scripts/surfaceproperties_manifest.txt` lists files of
//! `"name" { "key" "value" ... }` entries, read in order (later entries of
//! the same name override keys). `"base"` copies another entry first;
//! anything not set falls back to `"default"`. We keep the physics fields.

use std::collections::HashMap;

use super::material::MaterialLoader;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceProp {
    pub friction: f32,
    pub elasticity: f32,
}

#[derive(Default)]
pub struct SurfaceProps {
    /// Raw keys per entry (lower-case names), in definition order.
    entries: HashMap<String, HashMap<String, String>>,
}

/// Tokens: quoted strings, bare words and braces; `//` comments skipped.
pub(crate) fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                    s.push(c);
                }
                out.push(s);
            }
            '{' | '}' => out.push(c.to_string()),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            c => {
                let mut s = c.to_string();
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || n == '"' || n == '{' || n == '}' {
                        break;
                    }
                    s.push(n);
                    chars.next();
                }
                out.push(s);
            }
        }
    }
    out
}

impl SurfaceProps {
    /// Load the files the manifest lists (missing ones are skipped).
    pub fn load(materials: &mut MaterialLoader) -> Self {
        let mut me = Self::default();
        let Some(manifest) = materials.read("scripts/surfaceproperties_manifest.txt") else {
            return me;
        };
        let manifest = String::from_utf8_lossy(&manifest).to_string();
        let t = tokens(&manifest);
        let files: Vec<String> = t
            .windows(2)
            .filter(|w| w[0].eq_ignore_ascii_case("file"))
            .map(|w| w[1].clone())
            .collect();
        for f in files {
            if let Some(bytes) = materials.read(&f) {
                me.add(&String::from_utf8_lossy(&bytes));
            }
        }
        me
    }

    /// Add one file's entries.
    pub fn add(&mut self, text: &str) {
        let t = tokens(text);
        let mut i = 0;
        while i + 1 < t.len() {
            if t[i + 1] != "{" {
                i += 1;
                continue;
            }
            let name = t[i].to_lowercase();
            i += 2;
            let entry = self.entries.entry(name).or_default();
            while i < t.len() && t[i] != "}" {
                if i + 1 < t.len() {
                    entry.insert(t[i].to_lowercase(), t[i + 1].clone());
                }
                i += 2;
            }
            i += 1;
        }
    }

    fn key(&self, name: &str, key: &str, depth: u32) -> Option<f32> {
        let entry = self.entries.get(&name.to_lowercase())?;
        if let Some(v) = entry.get(key).and_then(|v| v.parse().ok()) {
            return Some(v);
        }
        let base = entry.get("base")?;
        (depth < 16).then(|| self.key(base, key, depth + 1)).flatten()
    }

    /// A text key, following `base` and then `default`.
    pub fn text(&self, name: &str, key: &str) -> Option<String> {
        fn find(me: &SurfaceProps, name: &str, key: &str, depth: u32) -> Option<String> {
            let entry = me.entries.get(&name.to_lowercase())?;
            if let Some(v) = entry.get(key) {
                return Some(v.clone());
            }
            let base = entry.get("base")?;
            (depth < 16).then(|| find(me, base, key, depth + 1)).flatten()
        }
        find(self, name, key, 0).or_else(|| find(self, "default", key, 0))
    }

    /// Every surface name (lower-case).
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }

    /// An entry's friction and elasticity (unknown names: `default`).
    pub fn get(&self, name: &str) -> SurfaceProp {
        let field = |key: &str, fallback: f32| {
            self.key(name, key, 0)
                .or_else(|| self.key("default", key, 0))
                .unwrap_or(fallback)
        };
        SurfaceProp {
            friction: field("friction", 0.8),
            elasticity: field("elasticity", 0.25),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_and_default() {
        let mut s = SurfaceProps::default();
        s.add(
            r#"
            "default" { "density" "2000" "elasticity" "0.25" "friction" "0.8" }
            // A comment.
            "Plastic_Box" { "density" "500" "elasticity" "0.01" }
            "plastic" { "base" "Plastic_Box" }
            "player" { "friction" "0.5" "elasticity" "0.001" }
            "#,
        );
        assert_eq!(
            s.get("plastic"),
            SurfaceProp {
                friction: 0.8,
                elasticity: 0.01
            }
        );
        assert_eq!(
            s.get("player"),
            SurfaceProp {
                friction: 0.5,
                elasticity: 0.001
            }
        );
        assert_eq!(
            s.get("nonexistent"),
            SurfaceProp {
                friction: 0.8,
                elasticity: 0.25
            }
        );
    }
}
