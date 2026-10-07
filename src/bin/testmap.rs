//! Dev tool: writes `mashup_logic_test.vmf`, a Hammer map with an example of
//! every entity class mashup supports, laid out in rooms along one sealed
//! hall (docs/plans/active/custom-maps.md, "Test map"). Our own box
//! geometry and stock CS:S/HL2 texture names only; Valve's compilers build
//! the `.bsp` (scripts/compile_testmap.ps1), which is never committed.
//!
//! Usage: `cargo run --bin testmap [-- out.vmf]` (default
//! tools/testmap/mashup_logic_test.vmf).
//!
//! Rooms along +x (Hammer units, z up), each with a visible result:
//! - A (x 0–512): entity I/O: logic_auto → logic_relay (1 s delay) → a
//!   func_brush appears; logic_timer toggles a brush every 2 s; a
//!   trigger_multiple counts into math_counter, whose OnHitMax shows a brush.
//! - B (512–1024): triggers: teleport, push up, push sideways, hurt, once.
//! - C (1024–1536): movers: func_door opened by a func_button,
//!   func_door_rotating (+use), func_movelinear platform on a button,
//!   func_rotating fan, func_tracktrain on four path_tracks.
//! - D (1536–2048): func_breakable glass/wood/metal, game_player_equip on a
//!   trigger, a placed weapon_awp, game_text on a trigger.
//! - E (2048–2560): spawns for both teams.

use std::fmt::Write as _;

const WALL: &str = "dev/dev_measurewall01a";
const FLOOR: &str = "dev/dev_measuregeneric01b";
const CRATE: &str = "dev/dev_measurecrate02";
const TRIGGER: &str = "tools/toolstrigger";
const GLASS: &str = "glass/glasswindow007a";
const WOOD: &str = "wood/woodwall009a";
const METAL: &str = "metal/metalwall001a";

/// Interior of the hall.
const HALL: ([f32; 3], [f32; 3]) = ([0.0, 0.0, 0.0], [2560.0, 768.0, 320.0]);
const THICK: f32 = 16.0;

/// An axis-aligned box brush: min, max corners and one material.
#[derive(Clone, Copy, Debug)]
struct Block {
    min: [f32; 3],
    max: [f32; 3],
    material: &'static str,
}

fn block(min: [f32; 3], max: [f32; 3], material: &'static str) -> Block {
    Block { min, max, material }
}

/// A point or brush entity: keyvalues, outputs (name, "target,input,param,delay,times") and brushes.
#[derive(Default)]
struct Entity {
    keys: Vec<(String, String)>,
    outputs: Vec<(String, String)>,
    brushes: Vec<Block>,
}

impl Entity {
    fn new(class: &str) -> Self {
        let mut e = Entity::default();
        e.keys.push(("classname".into(), class.into()));
        e
    }
    fn key(mut self, k: &str, v: impl ToString) -> Self {
        self.keys.push((k.into(), v.to_string()));
        self
    }
    fn at(self, p: [f32; 3]) -> Self {
        self.key("origin", format!("{} {} {}", p[0], p[1], p[2]))
    }
    /// An output: `target,input,param,delay,times`.
    fn out(mut self, output: &str, target: &str, input: &str, param: &str, delay: f32, times: i32) -> Self {
        self.outputs
            .push((output.into(), format!("{target},{input},{param},{delay},{times}")));
        self
    }
    fn brush(mut self, b: Block) -> Self {
        self.brushes.push(b);
        self
    }
}

/// The six faces of a box as Hammer writes them: three points each, in
/// the order that makes the compiler's plane normal point outward, with
/// texture axes.
fn faces(b: &Block) -> [([[f32; 3]; 3], [f32; 3], [f32; 3]); 6] {
    let ([x0, y0, z0], [x1, y1, z1]) = (b.min, b.max);
    let (ux, uy, nz) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]);
    let ny = [0.0, -1.0, 0.0];
    [
        ([[x0, y1, z1], [x1, y1, z1], [x1, y0, z1]], ux, ny), // top
        ([[x0, y0, z0], [x1, y0, z0], [x1, y1, z0]], ux, ny), // bottom
        ([[x0, y1, z1], [x0, y0, z1], [x0, y0, z0]], uy, nz), // -x
        ([[x1, y1, z0], [x1, y0, z0], [x1, y0, z1]], uy, nz), // +x
        ([[x1, y1, z1], [x0, y1, z1], [x0, y1, z0]], ux, nz), // +y
        ([[x1, y0, z0], [x0, y0, z0], [x0, y0, z1]], ux, nz), // -y
    ]
}

struct Writer {
    out: String,
    id: u32,
}

impl Writer {
    fn next(&mut self) -> u32 {
        self.id += 1;
        self.id
    }

    fn solid(&mut self, b: &Block, depth: usize) {
        let pad = "\t".repeat(depth);
        let id = self.next();
        let _ = writeln!(self.out, "{pad}solid\n{pad}{{\n{pad}\t\"id\" \"{id}\"");
        for (points, u, v) in faces(b) {
            let id = self.next();
            let p = points.map(|p| format!("({} {} {})", p[0], p[1], p[2])).join(" ");
            let _ = writeln!(
                self.out,
                "{pad}\tside\n{pad}\t{{\n{pad}\t\t\"id\" \"{id}\"\n{pad}\t\t\"plane\" \"{p}\"\n{pad}\t\t\"material\" \"{}\"\n{pad}\t\t\"uaxis\" \"[{} {} {} 0] 0.25\"\n{pad}\t\t\"vaxis\" \"[{} {} {} 0] 0.25\"\n{pad}\t\t\"rotation\" \"0\"\n{pad}\t\t\"lightmapscale\" \"16\"\n{pad}\t\t\"smoothing_groups\" \"0\"\n{pad}\t}}",
                b.material.to_uppercase(),
                u[0],
                u[1],
                u[2],
                v[0],
                v[1],
                v[2]
            );
        }
        let _ = writeln!(self.out, "{pad}}}");
    }

    fn entity(&mut self, e: &Entity) {
        let id = self.next();
        let _ = writeln!(self.out, "entity\n{{\n\t\"id\" \"{id}\"");
        for (k, v) in &e.keys {
            let _ = writeln!(self.out, "\t\"{k}\" \"{v}\"");
        }
        if !e.outputs.is_empty() {
            let _ = writeln!(self.out, "\tconnections\n\t{{");
            for (k, v) in &e.outputs {
                let _ = writeln!(self.out, "\t\t\"{k}\" \"{v}\"");
            }
            let _ = writeln!(self.out, "\t}}");
        }
        for b in &e.brushes {
            self.solid(b, 1);
        }
        let _ = writeln!(self.out, "}}");
    }
}

/// The hall's shell: floor, ceiling and four walls, overlapping at the
/// corners so the map is sealed.
fn shell() -> Vec<Block> {
    let ([x0, y0, z0], [x1, y1, z1]) = HALL;
    let t = THICK;
    vec![
        block([x0 - t, y0 - t, z0 - t], [x1 + t, y1 + t, z0], FLOOR),
        block([x0 - t, y0 - t, z1], [x1 + t, y1 + t, z1 + t], WALL),
        block([x0 - t, y0 - t, z0], [x0, y1 + t, z1], WALL),
        block([x1, y0 - t, z0], [x1 + t, y1 + t, z1], WALL),
        block([x0, y0 - t, z0], [x1, y0, z1], WALL),
        block([x0, y1, z0], [x1, y1 + t, z1], WALL),
    ]
}

/// A trigger volume on the floor: x and y extent, height.
fn pad(x: [f32; 2], y: [f32; 2], height: f32) -> Block {
    block([x[0], y[0], 0.0], [x[1], y[1], height], TRIGGER)
}

/// A crate "lamp" brush that logic shows or hides.
fn lamp(name: &str, x: f32, y: f32) -> Entity {
    Entity::new("func_brush")
        .key("targetname", name)
        .key("StartDisabled", 1)
        .key("Solidity", 1)
        .key("rendermode", 0)
        .brush(block([x, y, 64.0], [x + 32.0, y + 32.0, 96.0], CRATE))
}

fn entities() -> Vec<Entity> {
    let mut e = Vec::new();
    // --- A: entity I/O.
    e.push(Entity::new("logic_auto").at([32.0, 32.0, 16.0]).out("OnMapSpawn", "relay_a", "Trigger", "", 1.0, -1));
    e.push(Entity::new("logic_relay").key("targetname", "relay_a").at([64.0, 32.0, 16.0]).out(
        "OnTrigger",
        "lamp_a1",
        "Enable",
        "",
        0.0,
        -1,
    ));
    e.push(lamp("lamp_a1", 96.0, 640.0));
    e.push(
        Entity::new("logic_timer")
            .key("targetname", "timer_a")
            .key("RefireTime", 2)
            .key("StartDisabled", 0)
            .at([96.0, 32.0, 16.0])
            .out("OnTimer", "lamp_a2", "Toggle", "", 0.0, -1),
    );
    e.push(lamp("lamp_a2", 224.0, 640.0));
    e.push(
        Entity::new("math_counter")
            .key("targetname", "count_a")
            .key("startvalue", 0)
            .key("min", 0)
            .key("max", 3)
            .at([128.0, 32.0, 16.0])
            .out("OnHitMax", "lamp_a3", "Enable", "", 0.0, -1),
    );
    e.push(lamp("lamp_a3", 352.0, 640.0));
    e.push(
        Entity::new("trigger_multiple")
            .key("spawnflags", 1)
            .key("wait", 1)
            .brush(pad([320.0, 448.0], [192.0, 320.0], 128.0))
            .out("OnStartTouch", "count_a", "Add", "1", 0.0, -1),
    );
    // --- B: triggers.
    e.push(
        Entity::new("trigger_teleport")
            .key("spawnflags", 1)
            .key("target", "dest_b")
            .brush(pad([560.0, 656.0], [64.0, 160.0], 128.0)),
    );
    e.push(
        Entity::new("info_teleport_destination")
            .key("targetname", "dest_b")
            .key("angles", "0 180 0")
            .at([960.0, 640.0, 16.0]),
    );
    e.push(
        Entity::new("trigger_push")
            .key("spawnflags", 1)
            .key("pushdir", "-90 0 0")
            .key("speed", 600)
            .brush(pad([688.0, 784.0], [64.0, 160.0], 288.0)),
    );
    e.push(
        Entity::new("trigger_push")
            .key("spawnflags", 1)
            .key("pushdir", "0 90 0")
            .key("speed", 300)
            .brush(pad([816.0, 912.0], [64.0, 160.0], 128.0)),
    );
    e.push(
        Entity::new("trigger_hurt")
            .key("spawnflags", 1)
            .key("damage", 10)
            .key("damagetype", 0)
            .brush(pad([560.0, 656.0], [320.0, 416.0], 64.0)),
    );
    e.push(
        Entity::new("trigger_once")
            .key("spawnflags", 1)
            .brush(pad([688.0, 784.0], [320.0, 416.0], 128.0))
            .out("OnTrigger", "lamp_b1", "Enable", "", 0.0, -1),
    );
    e.push(lamp("lamp_b1", 720.0, 640.0));
    // --- C: movers.
    e.push(
        Entity::new("func_door")
            .key("targetname", "door_c")
            .key("movedir", "-90 0 0")
            .key("speed", 100)
            .key("wait", 3)
            .key("lip", 8)
            .key("spawnflags", 0)
            .brush(block([1088.0, 256.0, 0.0], [1104.0, 384.0, 128.0], METAL)),
    );
    e.push(
        Entity::new("func_button")
            .key("spawnflags", 1025)
            .key("speed", 5)
            .key("wait", 1)
            .brush(block([1056.0, 200.0, 56.0], [1072.0, 216.0, 72.0], CRATE))
            .out("OnPressed", "door_c", "Open", "", 0.0, -1),
    );
    e.push(
        Entity::new("func_door_rotating")
            .key("targetname", "rotdoor_c")
            .key("distance", 90)
            .key("speed", 100)
            .key("wait", -1)
            .key("spawnflags", 256)
            .at([1216.0, 512.0, 64.0])
            .brush(block([1216.0, 512.0, 0.0], [1224.0, 608.0, 128.0], WOOD)),
    );
    e.push(
        Entity::new("func_movelinear")
            .key("targetname", "plat_c")
            .key("movedir", "-90 0 0")
            .key("movedistance", 128)
            .key("speed", 64)
            .brush(block([1344.0, 96.0, 0.0], [1472.0, 224.0, 16.0], METAL)),
    );
    e.push(
        Entity::new("func_button")
            .key("spawnflags", 1025)
            .key("speed", 5)
            .key("wait", 1)
            .brush(block([1312.0, 48.0, 56.0], [1328.0, 64.0, 72.0], CRATE))
            .out("OnPressed", "plat_c", "Open", "", 0.0, -1)
            .out("OnPressed", "plat_c", "Close", "", 4.0, -1),
    );
    e.push(
        Entity::new("func_rotating")
            .key("spawnflags", 1)
            .key("maxspeed", 100)
            .at([1440.0, 640.0, 200.0])
            .brush(block([1376.0, 632.0, 196.0], [1504.0, 648.0, 204.0], METAL)),
    );
    let path = [
        ("path_c1", [1088.0, 704.0, 16.0], "path_c2"),
        ("path_c2", [1280.0, 704.0, 16.0], "path_c3"),
        ("path_c3", [1280.0, 448.0, 16.0], "path_c4"),
        ("path_c4", [1088.0, 448.0, 16.0], "path_c1"),
    ];
    for (name, at, next) in path {
        e.push(Entity::new("path_track").key("targetname", name).key("target", next).at(at));
    }
    e.push(
        Entity::new("func_tracktrain")
            .key("targetname", "train_c")
            .key("target", "path_c1")
            .key("speed", 64)
            .key("startspeed", 64)
            .key("spawnflags", 2)
            .at([1088.0, 704.0, 24.0])
            .brush(block([1056.0, 672.0, 16.0], [1120.0, 736.0, 32.0], CRATE)),
    );
    // --- D: breakables and equipment.
    for (i, (material, mat_num, health)) in [(GLASS, 0, 1), (WOOD, 1, 50), (METAL, 2, 1)].into_iter().enumerate() {
        let x = 1600.0 + i as f32 * 96.0;
        e.push(
            Entity::new("func_breakable")
                .key("material", mat_num)
                .key("health", health)
                .key("spawnflags", 0)
                .brush(block([x, 320.0, 0.0], [x + 8.0, 448.0, 96.0], material)),
        );
    }
    e.push(
        Entity::new("game_player_equip")
            .key("targetname", "equip_d")
            .key("spawnflags", 1)
            .key("weapon_ak47", 1)
            .key("item_kevlar", 1)
            .at([1904.0, 32.0, 16.0]),
    );
    e.push(
        Entity::new("trigger_once")
            .key("spawnflags", 1)
            .brush(pad([1856.0, 1952.0], [64.0, 160.0], 128.0))
            .out("OnStartTouch", "equip_d", "Use", "", 0.0, -1),
    );
    e.push(Entity::new("weapon_awp").key("angles", "0 90 0").at([1904.0, 640.0, 16.0]));
    e.push(
        Entity::new("game_text")
            .key("targetname", "text_d")
            .key("message", "mashup logic test: room D")
            .key("x", -1)
            .key("y", 0.3)
            .key("holdtime", 3)
            .key("fadein", 0.5)
            .key("fadeout", 0.5)
            .key("color", "255 176 0")
            .key("channel", 1)
            .at([1984.0, 32.0, 16.0]),
    );
    e.push(
        Entity::new("trigger_multiple")
            .key("spawnflags", 1)
            .key("wait", 4)
            .brush(pad([1856.0, 1952.0], [256.0, 352.0], 128.0))
            .out("OnStartTouch", "text_d", "Display", "", 0.0, -1),
    );
    // --- E: spawns, then lights along the hall.
    for y in [256.0, 512.0] {
        e.push(Entity::new("info_player_counterterrorist").key("angles", "0 180 0").at([2304.0, y, 16.0]));
        e.push(Entity::new("info_player_terrorist").key("angles", "0 180 0").at([2432.0, y, 16.0]));
    }
    let mut x = 128.0;
    while x < HALL.1[0] {
        for y in [192.0, 576.0] {
            e.push(Entity::new("light").key("_light", "255 250 235 250").at([x, y, 288.0]));
        }
        x += 256.0;
    }
    e
}

fn vmf() -> String {
    let mut w = Writer {
        out: String::new(),
        id: 0,
    };
    w.out.push_str("versioninfo\n{\n\t\"editorversion\" \"400\"\n\t\"editorbuild\" \"8000\"\n\t\"mapversion\" \"1\"\n\t\"formatversion\" \"100\"\n\t\"prefab\" \"0\"\n}\n");
    let world = w.next();
    let _ = writeln!(
        w.out,
        "world\n{{\n\t\"id\" \"{world}\"\n\t\"mapversion\" \"1\"\n\t\"classname\" \"worldspawn\"\n\t\"skyname\" \"sky_day01_01\""
    );
    for b in shell() {
        w.solid(&b, 1);
    }
    w.out.push_str("}\n");
    for e in entities() {
        w.entity(&e);
    }
    w.out.push_str("cameras\n{\n\t\"activecamera\" \"-1\"\n}\n");
    w.out
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tools/testmap/mashup_logic_test.vmf".into());
    if let Some(dir) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(dir).expect("output folder");
    }
    std::fs::write(&path, vmf()).expect("write vmf");
    println!("wrote {path}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    }

    #[test]
    fn box_faces_point_outward() {
        // The compiler's plane normal: (p0 - p1) x (p2 - p1).
        let b = block([0.0, 0.0, 0.0], [64.0, 32.0, 16.0], WALL);
        let centre = [32.0, 16.0, 8.0];
        for (p, ..) in faces(&b) {
            let n = cross(sub(p[0], p[1]), sub(p[2], p[1]));
            let out = sub(p[1], centre);
            let d = n[0] * out[0] + n[1] * out[1] + n[2] * out[2];
            assert!(d > 0.0, "face {p:?} normal {n:?} points inward");
        }
    }

    #[test]
    fn every_supported_class_is_in_the_map() {
        let text = vmf();
        for class in [
            "logic_auto",
            "logic_relay",
            "logic_timer",
            "math_counter",
            "trigger_multiple",
            "trigger_once",
            "trigger_teleport",
            "info_teleport_destination",
            "trigger_push",
            "trigger_hurt",
            "func_door",
            "func_door_rotating",
            "func_button",
            "func_movelinear",
            "func_rotating",
            "func_tracktrain",
            "path_track",
            "func_breakable",
            "func_brush",
            "game_player_equip",
            "game_text",
            "info_player_counterterrorist",
            "info_player_terrorist",
        ] {
            assert!(text.contains(&format!("\"classname\" \"{class}\"")), "{class} missing");
        }
        // Braces balance.
        assert_eq!(text.matches('{').count(), text.matches('}').count());
    }

    #[test]
    fn committed_vmf_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/testmap/mashup_logic_test.vmf");
        // Git on Windows may check it out with CRLF line endings.
        let committed = std::fs::read_to_string(path).unwrap_or_default().replace('\r', "");
        assert!(
            committed == vmf(),
            "tools/testmap/mashup_logic_test.vmf is out of date: run `cargo run --bin testmap`"
        );
    }
}
