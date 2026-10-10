//! CS:S sound scripts (specs/cs_source/sounds.md section 1): the
//! game_sounds manifest's files and the map's own `maps/<map>_level_sounds.txt`
//! define named entries (waves, volume, pitch, sound level or
//! attenuation, channel). A map loads the entries it uses, decoding their
//! waves once (like Source's precache).

use std::collections::{HashMap, HashSet};

use super::{
    material::MaterialLoader,
    surfaceprops::{SurfaceProps, tokens},
};
use crate::map::{MapEntity, MapSoundEntry, MapSounds, MapSurface, SoundLevel, sound::Interval};

/// An entry as written: its keys, and its waves in order.
#[derive(Clone, Debug, Default)]
struct RawEntry {
    keys: HashMap<String, String>,
    waves: Vec<String>,
}

/// All script entries, later files overriding earlier ones.
#[derive(Default)]
pub struct SoundScripts {
    entries: HashMap<String, RawEntry>,
}

impl SoundScripts {
    pub fn load(materials: &mut MaterialLoader, map: &str) -> Self {
        let mut me = Self::default();
        if let Some(manifest) = materials.read("scripts/game_sounds_manifest.txt") {
            let t = tokens(&String::from_utf8_lossy(&manifest));
            let files: Vec<String> = t
                .windows(2)
                .filter(|w| w[0].eq_ignore_ascii_case("precache_file") || w[0].eq_ignore_ascii_case("preload_file"))
                .map(|w| w[1].clone())
                .collect();
            for f in files {
                if let Some(b) = materials.read(&f) {
                    me.add(&String::from_utf8_lossy(&b));
                }
            }
        }
        if let Some(b) = materials.read(&format!("maps/{}_level_sounds.txt", map.to_lowercase())) {
            me.add(&String::from_utf8_lossy(&b));
        }
        me
    }

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
            let mut entry = RawEntry::default();
            while i < t.len() && t[i] != "}" {
                let key = t[i].to_lowercase();
                if t.get(i + 1).map(String::as_str) == Some("{") {
                    // rndwave { "wave" ... }
                    i += 2;
                    while i < t.len() && t[i] != "}" {
                        if t[i].eq_ignore_ascii_case("wave") && i + 1 < t.len() {
                            entry.waves.push(t[i + 1].clone());
                        }
                        i += 2;
                    }
                    i += 1;
                    continue;
                }
                let Some(value) = t.get(i + 1).cloned() else { break };
                if key == "wave" {
                    entry.waves.push(value);
                } else {
                    entry.keys.insert(key, value);
                }
                i += 2;
            }
            i += 1;
            self.entries.insert(name, entry);
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(&name.to_lowercase())
    }
}

/// "a" or "a,b": start a, range b - a (C-style number prefixes).
pub(super) fn interval(text: &str) -> Interval {
    let num = |s: &str| -> f32 {
        let s = s.trim();
        let end = s
            .char_indices()
            .find(|(i, c)| !(c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '-' || *c == '+'))))
            .map_or(s.len(), |(i, _)| i);
        s[..end].parse().unwrap_or(0.0)
    };
    match text.split_once(',') {
        Some((a, b)) => Interval {
            start: num(a),
            range: num(b) - num(a),
        },
        None => Interval::fixed(num(text)),
    }
}

pub(super) fn named_level(text: &str) -> Option<f32> {
    let rest = text.to_ascii_uppercase();
    let rest = rest.strip_prefix("SNDLVL_")?;
    Some(match rest {
        "NONE" => 0.0,
        "IDLE" => 60.0,
        "TALKING" => 80.0,
        "STATIC" => 66.0,
        "NORM" => 75.0,
        "GUNFIRE" => 140.0,
        other => {
            let digits: String = other.chars().take_while(char::is_ascii_digit).collect();
            match digits.parse::<f32>() {
                Ok(v) if (1.0..=180.0).contains(&v) => v,
                _ => 75.0,
            }
        }
    })
}

pub(super) fn attenuation(text: &str) -> f32 {
    match text.trim().to_ascii_uppercase().as_str() {
        "ATTN_NONE" => 0.0,
        "ATTN_NORM" => 0.8,
        "ATTN_IDLE" => 2.0,
        "ATTN_STATIC" => 1.25,
        "ATTN_RICOCHET" => 1.5,
        "ATTN_GUNFIRE" => 0.27,
        other => interval(other).start,
    }
}

fn channel(text: &str) -> u8 {
    let t = text.trim().to_ascii_uppercase();
    match t.as_str() {
        "CHAN_AUTO" => 0,
        "CHAN_WEAPON" => 1,
        "CHAN_VOICE" => 2,
        "CHAN_ITEM" => 3,
        "CHAN_BODY" => 4,
        "CHAN_STREAM" => 5,
        "CHAN_STATIC" => 6,
        "CHAN_VOICE2" => 7,
        _ if t.starts_with("CHAN_") => 0,
        _ => t.parse().unwrap_or(0),
    }
}

const WAVE_PREFIXES: [char; 10] = ['*', '#', ')', '^', '<', '>', '@', '}', '!', '?'];

/// A wave path as a file: prefix characters stripped, under `sound/`.
pub(super) fn wave_file(wave: &str) -> String {
    let path = wave.trim_start_matches(WAVE_PREFIXES);
    format!("sound/{}", path.replace('\\', "/"))
}

/// A wave marked `#` among its prefix characters bypasses the room DSP.
pub(super) fn wave_dry(wave: &str) -> bool {
    wave.chars().take_while(|c| WAVE_PREFIXES.contains(c)).any(|c| c == '#')
}

/// Entries the movement and world use, beyond the surfaces' steps.
/// func_breakable's material sounds (logic::breakables::Material).
const BREAKABLE_SOUNDS: &[&str] = &[
    "Breakable.Glass",
    "Breakable.Crate",
    "Breakable.Metal",
    "Breakable.Flesh",
    "Breakable.Concrete",
    "Breakable.Ceiling",
    "Breakable.MatGlass",
    "Breakable.MatWood",
    "Breakable.MatMetal",
    "Breakable.MatConcrete",
    "Breakable.Computer",
    // Their gibs' bounces (Material::bounce_sound).
    "Bounce.Glass",
    "Bounce.Wood",
    "Bounce.Metal",
    "Bounce.Flesh",
    "Bounce.Concrete",
];

const ALWAYS: &[&str] = &[
    "Player.Swim",
    "Event.TERWin",
    "Event.CTWin",
    "Event.RoundDraw",
    "radio.moveout",
    "radio.letsgo",
    "radio.locknload",
    // +use on nothing, a chat line, buying ammo.
    "Player.UseDeny",
    super::radio::CHAT_SOUND,
    super::weapons::AMMO_SOUND,
];

/// The announcer's round sounds (game_sounds_radio.txt entries).
pub fn round_sounds() -> crate::map::RoundSounds {
    crate::map::RoundSounds {
        attackers_win: Some("Event.TERWin".into()),
        defenders_win: Some("Event.CTWin".into()),
        draw: Some("Event.RoundDraw".into()),
        start: ["radio.moveout", "radio.letsgo", "radio.locknload"].map(String::from).to_vec(),
        // The install defines it twice (common/use_deny.wav, then
        // common/wpn_select.wav at 0.4); the later definition wins here.
        // Which CS:S plays is an open question (doors_buttons.md Q2).
        use_deny: Some("Player.UseDeny".into()),
    }
}

/// What the map's ambient_generics play ("message"): script entries, and
/// raw wave files (anything naming a .wav or .mp3). Sentences ("!...")
/// are left out.
pub fn ambient_messages(entities: &[MapEntity]) -> (Vec<String>, Vec<String>) {
    let (mut entries, mut raw) = (Vec::new(), Vec::new());
    // Their own messages, and ones outputs set at run time (an
    // `AddOutput message <sound>` connection: mg_crazykart_v1_1).
    let own = entities
        .iter()
        .filter(|e| e.classname().eq_ignore_ascii_case("ambient_generic"))
        .filter_map(|e| e.get("message"));
    let added = entities.iter().flat_map(|e| e.keyvalues.iter()).filter_map(|(_, v)| {
        let mut f = v.split(['\u{1b}', ',']);
        let _target = f.next()?;
        f.next()?.trim().eq_ignore_ascii_case("addoutput").then_some(())?;
        let param = f.next()?.trim();
        param
            .get(..8)
            .is_some_and(|k| k.eq_ignore_ascii_case("message "))
            .then(|| &param[8..])
    });
    for m in own.chain(added) {
        let m = m.trim();
        if m.is_empty() || m.starts_with('!') {
            continue;
        }
        let lower = m.to_lowercase();
        let list = if lower.contains(".wav") || lower.contains(".mp3") {
            &mut raw
        } else {
            &mut entries
        };
        if !list.contains(&lower) {
            list.push(lower);
        }
    }
    (entries, raw)
}

/// The map's sounds: the entries it uses and their decoded waves, plus the
/// surfaces' step sounds and game materials. The map's ambient_generic
/// sounds are loaded too, a raw wave as an entry named by its path
/// (volume 1, pitch 100, level 75: the entity gives its own).
pub fn load(materials: &mut MaterialLoader, map: &str, surfaces: &SurfaceProps, entities: &[MapEntity]) -> MapSounds {
    let scripts = SoundScripts::load(materials, map);
    let mut out = MapSounds::default();
    let (ambient_entries, ambient_raw) = ambient_messages(entities);
    let mut wanted: HashSet<String> = ALWAYS
        .iter()
        .chain(super::weapons::sounds().iter())
        .copied()
        .chain(super::radio::sound_entries())
        .chain(super::fire::SOUNDS.iter().copied())
        .chain(super::pain::SOUNDS.iter().copied())
        .map(|s| s.to_lowercase())
        .chain(ambient_entries)
        // Props' break and explosion sounds (props.rs), breakables' own.
        .chain(entities.iter().flat_map(|e| {
            e.keyvalues
                .iter()
                .filter(|(k, _)| {
                    k == crate::map::entities::PROP_BREAK_SOUND_KEY || k == crate::map::entities::PROP_EXPLODE_SOUND_KEY
                })
                .map(|(_, v)| v.to_lowercase())
        }))
        .chain(BREAKABLE_SOUNDS.iter().map(|s| s.to_lowercase()))
        .collect();
    for name in surfaces.names() {
        let num = |key: &str, fallback: f32| {
            surfaces
                .text(name, key)
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(fallback)
        };
        let surface = MapSurface {
            step_left: surfaces.text(name, "stepleft"),
            step_right: surfaces.text(name, "stepright"),
            game_material: surfaces
                .text(name, "gamematerial")
                .and_then(|g| g.chars().next())
                .unwrap_or('C')
                .to_ascii_uppercase(),
            bullet_impact: surfaces.text(name, "bulletimpact"),
            impact_soft: surfaces.text(name, "impactsoft"),
            impact_hard: surfaces.text(name, "impacthard"),
            hardness: num("audiohardnessfactor", 1.0),
            hard_threshold: num("impacthardthreshold", 0.5),
            hard_min_velocity: num("audiohardminvelocity", 0.0),
            scrape_rough: surfaces.text(name, "scraperough"),
            scrape_smooth: surfaces.text(name, "scrapesmooth"),
            roughness: num("audioroughnessfactor", 1.0),
            rough_threshold: num("scraperoughthreshold", 0.5),
        };
        wanted.extend(
            [
                &surface.step_left,
                &surface.step_right,
                &surface.bullet_impact,
                &surface.impact_soft,
                &surface.impact_hard,
                &surface.scrape_rough,
                &surface.scrape_smooth,
            ]
            .into_iter()
            .flatten()
            .map(|s| s.to_lowercase()),
        );
        out.surfaces.insert(name.clone(), surface);
    }
    let mut decoded: HashMap<String, Option<usize>> = HashMap::new();
    let mut failed = 0;
    for name in wanted {
        let Some(raw) = scripts.entries.get(&name) else {
            continue;
        };
        let mut waves = Vec::new();
        for w in &raw.waves {
            let file = wave_file(w);
            let index = *decoded.entry(file.clone()).or_insert_with(|| {
                let clip = materials.read(&file).and_then(|b| super::wav::decode_any(&b).ok());
                clip.map(|c| {
                    out.clips.push(c);
                    out.clips.len() - 1
                })
            });
            match index {
                Some(i) => waves.push(i),
                None => failed += 1,
            }
        }
        let keys = &raw.keys;
        let level = match (
            keys.get("soundlevel"),
            keys.get("compatibilityattenuation"),
            keys.get("attenuation"),
        ) {
            (Some(l), ..) => SoundLevel::Db(named_level(l).unwrap_or_else(|| interval(l).start)),
            (None, Some(a), _) | (None, None, Some(a)) => SoundLevel::Attenuation(attenuation(a)),
            _ => SoundLevel::Db(75.0),
        };
        let volume = keys.get("volume").map_or(Interval::fixed(1.0), |v| {
            if v.eq_ignore_ascii_case("VOL_NORM") {
                Interval::fixed(1.0)
            } else {
                interval(v)
            }
        });
        let pitch = keys
            .get("pitch")
            .map_or(Interval::fixed(100.0), |p| match p.to_ascii_uppercase().as_str() {
                "PITCH_NORM" => Interval::fixed(100.0),
                "PITCH_LOW" => Interval::fixed(95.0),
                "PITCH_HIGH" => Interval::fixed(120.0),
                _ => interval(p),
            });
        out.entries.insert(
            name,
            MapSoundEntry {
                waves,
                volume,
                pitch,
                level,
                channel: keys.get("channel").map_or(0, |c| channel(c)),
                dry: !raw.waves.is_empty() && raw.waves.iter().all(|w| wave_dry(w)),
            },
        );
    }
    for name in ambient_raw {
        let file = wave_file(&name);
        let index = *decoded.entry(file.clone()).or_insert_with(|| {
            let clip = materials.read(&file).and_then(|b| super::wav::decode_any(&b).ok());
            clip.map(|c| {
                out.clips.push(c);
                out.clips.len() - 1
            })
        });
        let Some(index) = index else {
            failed += 1;
            continue;
        };
        let dry = wave_dry(&name);
        out.entries.entry(name).or_insert(MapSoundEntry {
            waves: vec![index],
            volume: Interval::fixed(1.0),
            pitch: Interval::fixed(100.0),
            level: SoundLevel::Db(75.0),
            channel: 6,
            dry,
        });
    }
    if failed > 0 {
        bevy::log::warn!("sounds: {failed} waves couldn't be read or decoded");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_outputs_set_on_ambients_are_loaded() {
        let kv = |pairs: &[(&str, &str)]| MapEntity {
            keyvalues: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            ..Default::default()
        };
        let ents = [
            kv(&[("classname", "ambient_generic"), ("message", "Ambient.Wind")]),
            kv(&[
                ("classname", "logic_relay"),
                ("OnTrigger", "snd,AddOutput,message kart/boost.mp3,0,-1"),
                ("OnUser1", "snd\u{1b}AddOutput\u{1b}message Kart.Hit\u{1b}0\u{1b}-1"),
            ]),
        ];
        let (entries, raw) = ambient_messages(&ents);
        assert_eq!(entries, ["ambient.wind", "kart.hit"]);
        assert_eq!(raw, ["kart/boost.mp3"]);
    }

    #[test]
    fn hash_prefix_marks_dry_waves() {
        assert!(wave_dry("#music/hl2_song1.wav"));
        assert!(wave_dry(")#weapons/x.wav"));
        assert!(!wave_dry(")weapons/ak47/ak47-1.wav"));
        assert!(!wave_dry("ambient/a#b.wav"));
        assert_eq!(wave_file("*#music/a.wav"), "sound/music/a.wav");
    }

    #[test]
    fn intervals_and_names() {
        assert_eq!(
            interval("95, 105"),
            Interval {
                start: 95.0,
                range: 10.0
            }
        );
        assert_eq!(
            interval("15,40`"),
            Interval {
                start: 15.0,
                range: 25.0
            }
        );
        assert_eq!(
            interval("105,95"),
            Interval {
                start: 105.0,
                range: -10.0
            }
        );
        assert_eq!(named_level("SNDLVL_140db"), Some(140.0));
        assert_eq!(named_level("SNDLVL_97dB"), Some(97.0));
        assert_eq!(named_level("SNDLVL_NORM"), Some(75.0));
        assert_eq!(attenuation("ATTN_NORM"), 0.8);
        assert_eq!(channel("chan_voice"), 2);
        assert_eq!(wave_file(")weapons/ak47/ak47-1.wav"), "sound/weapons/ak47/ak47-1.wav");
    }

    #[test]
    fn entries_with_rndwave() {
        let mut s = SoundScripts::default();
        s.add(
            r#""Player.Swim" { "channel" "CHAN_BODY" "volume" "VOL_NORM"
               "CompatibilityAttenuation" "1.0" "pitch" "PITCH_NORM"
               "rndwave" { "wave" "player/footsteps/slosh1.wav" "wave" "player/footsteps/slosh2.wav" } }"#,
        );
        let e = &s.entries["player.swim"];
        assert_eq!(e.waves.len(), 2);
        assert_eq!(e.keys["channel"], "CHAN_BODY");
    }
}
