//! The console's core (docs/plans/active/console.md): a registry of cvars
//! and commands any system can add to, a command queue with Source-style
//! syntax (`;` separates commands, quotes group words, `+`/`-` actions for
//! binds), aliases, binds, `wait`, and output lines with a severity. The
//! client draws it and feeds it keys; tests and the remote protocol queue
//! lines directly. Everything runs in one exclusive system each frame.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use bevy::prelude::*;

pub type RunFn = Arc<dyn Fn(&mut World, &[String]) -> Result<Option<String>, String> + Send + Sync>;
pub type GetFn = Arc<dyn Fn(&mut World) -> Option<String> + Send + Sync>;
pub type SetFn = Arc<dyn Fn(&mut World, &str) -> Result<(), String> + Send + Sync>;
pub type CompleteFn = Arc<dyn Fn(&World, &str) -> Vec<String> + Send + Sync>;

/// A console variable: read and written through closures, so it can live
/// in any resource or component.
#[derive(Clone)]
pub struct Cvar {
    pub name: String,
    pub help: String,
    pub default: String,
    /// Values to offer for completion (enums, booleans).
    pub values: Vec<String>,
    /// Saved to config.cfg (Source's FCVAR_ARCHIVE): player preferences,
    /// not test or server settings.
    pub archive: bool,
    /// Sensible bounds (Source's FCVAR min/max, here only a hint): the
    /// console's argument help shows them, the debug UI's sliders use them.
    pub range: Option<(f32, f32)>,
    /// Takes whole numbers only (a field of an integer type).
    pub integer: bool,
    /// Who owns it in a network game (from its name, `CvarScope::of`).
    pub scope: CvarScope,
    pub get: GetFn,
    pub set: SetFn,
}

/// Who owns a cvar in a network game (docs/plans/active/multiplayer.md,
/// "Console and cvars"): a network client can't set the server's
/// (`execute` refuses), and gets the replicated ones' values from the
/// server (`net::cvars`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CvarScope {
    /// The player's own (`cl_*`, `m_*`, video, audio, `mashup_*` tools).
    Local,
    /// The server's, never sent to clients (bots, secrets).
    Server,
    /// The server's, sent to clients on connect and on change (Source's
    /// FCVAR_REPLICATED): movement and game rules clients predict or show.
    Replicated,
}

/// Name prefixes of server and game-rule settings: movement and physics
/// `sv_*`, round rules `mp_*`, `phys_*`, `bot_*`, `ammo_*`. Maps may set
/// these through point_servercommand (`logic::classes::SETTING_PREFIXES`).
pub const SERVER_PREFIXES: &[&str] = &["sv_", "mp_", "phys_", "bot_", "ammo_"];

/// Game rules of ours without a server prefix that are the server's and
/// replicated all the same.
pub const SERVER_NAMES: &[&str] = &["mashup_rounds"];

impl CvarScope {
    /// The scope of a cvar named `name`: the server's when it has a server
    /// prefix; of those, bots' settings and secrets stay on the server and
    /// the rest replicate.
    pub fn of(name: &str) -> Self {
        let name = name.to_lowercase();
        if SERVER_NAMES.contains(&name.as_str()) {
            CvarScope::Replicated
        } else if !SERVER_PREFIXES.iter().any(|p| name.starts_with(p)) {
            CvarScope::Local
        } else if name.starts_with("bot_") || name.contains("password") || name.contains("rcon") {
            CvarScope::Server
        } else {
            CvarScope::Replicated
        }
    }
}

#[derive(Clone)]
pub struct Command {
    pub name: String,
    pub help: String,
    pub run: RunFn,
    /// Completion for the arguments (the text after the name).
    pub complete: Option<CompleteFn>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// A line the user typed.
    Input,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub text: String,
    pub level: Level,
    /// Seconds since startup.
    pub time: f64,
}

/// The console's state.
#[derive(Resource, Default)]
pub struct Console {
    cvars: BTreeMap<String, Cvar>,
    commands: BTreeMap<String, Command>,
    pub aliases: BTreeMap<String, String>,
    /// Key name (lower-case) -> command line.
    pub binds: BTreeMap<String, String>,
    pub output: Vec<Line>,
    /// Lines waiting to run, and frames to wait before the next.
    queue: VecDeque<String>,
    wait: u32,
    /// Expressions shown on the HUD (`watch`).
    pub watches: Vec<String>,
    /// Bumped when something should be saved (binds, cvars changed).
    pub dirty: bool,
    /// Lines printed since startup (the output keeps only the last ones).
    pub printed: u64,
}

const MAX_OUTPUT: usize = 4000;
const MAX_ALIAS_DEPTH: usize = 32;

impl Console {
    pub fn add_cvar(&mut self, cvar: Cvar) {
        self.cvars.insert(cvar.name.to_lowercase(), cvar);
    }

    /// Mark a cvar to be saved in config.cfg.
    pub fn archive(&mut self, name: &str) {
        if let Some(c) = self.cvars.get_mut(&name.to_lowercase()) {
            c.archive = true;
        }
    }

    /// Give a cvar a range for hints and sliders.
    pub fn set_range(&mut self, name: &str, min: f32, max: f32) {
        if let Some(c) = self.cvars.get_mut(&name.to_lowercase()) {
            c.range = Some((min, max));
        }
    }

    /// Give a command argument completion.
    pub fn set_completion(&mut self, name: &str, complete: CompleteFn) {
        if let Some(c) = self.commands.get_mut(&name.to_lowercase()) {
            c.complete = Some(complete);
        }
    }

    pub fn add_command(&mut self, command: Command) {
        self.commands.insert(command.name.to_lowercase(), command);
    }

    pub fn cvar(&self, name: &str) -> Option<&Cvar> {
        self.cvars.get(&name.to_lowercase())
    }

    pub fn command(&self, name: &str) -> Option<&Command> {
        self.commands.get(&name.to_lowercase())
    }

    pub fn cvars(&self) -> impl Iterator<Item = &Cvar> {
        self.cvars.values()
    }

    pub fn commands(&self) -> impl Iterator<Item = &Command> {
        self.commands.values()
    }

    /// Queue a line (it may hold several `;`-separated commands).
    pub fn submit(&mut self, line: impl Into<String>) {
        self.queue.push_back(line.into());
    }

    pub fn print(&mut self, level: Level, text: impl Into<String>) {
        let time = self.output.last().map_or(0.0, |l| l.time);
        for t in text.into().lines() {
            self.printed += 1;
            self.output.push(Line {
                text: t.to_string(),
                level,
                time,
            });
        }
        if self.output.len() > MAX_OUTPUT {
            let extra = self.output.len() - MAX_OUTPUT;
            self.output.drain(..extra);
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.print(Level::Info, text);
    }

    /// Every name the console knows: cvars, commands, aliases.
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .cvars
            .keys()
            .chain(self.commands.keys())
            .chain(self.aliases.keys())
            .cloned()
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

/// Split a line into commands at `;` (outside quotes), and each command
/// into words (quotes group words and are removed; `//` starts a comment).
pub fn parse(line: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut in_word) = (false, false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_word = true;
            }
            '/' if !quoted && chars.peek() == Some(&'/') => break,
            ';' if !quoted => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
                if !words.is_empty() {
                    commands.push(std::mem::take(&mut words));
                }
            }
            c if c.is_whitespace() && !quoted => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    if !words.is_empty() {
        commands.push(words);
    }
    commands
}

/// Where the last command of a line starts (after its last `;` outside
/// quotes, and the spaces after it): completion and argument help work
/// on that command.
pub fn last_command_start(line: &str) -> usize {
    let mut quoted = false;
    let mut start = 0;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => start = i + 1,
            _ => {}
        }
    }
    start + line[start..].len() - line[start..].trim_start().len()
}

/// Which argument of a command is being typed (0: the first after the
/// name), or None while the name itself is.
pub fn arg_position(command: &str) -> Option<usize> {
    let words = command.split_whitespace().count();
    let ends_space = command.ends_with(char::is_whitespace);
    match (words, ends_space) {
        (0, _) | (1, false) => None,
        (n, true) => Some(n - 1),
        (n, false) => Some(n - 2),
    }
}

/// A command's usage from the start of its help, Source style
/// (`"setpos <x> <y> <z>: move there"`): the argument words (brackets
/// kept, so `[amount=100]` is optional) and the description after the
/// colon. Help that doesn't start with the name has no usage.
pub fn usage(name: &str, help: &str) -> (Vec<String>, String) {
    let Some(rest) = help
        .get(..name.len())
        .filter(|h| h.eq_ignore_ascii_case(name))
        .map(|_| &help[name.len()..])
        .filter(|r| r.starts_with(' ') || r.starts_with(':'))
    else {
        return (Vec::new(), help.to_string());
    };
    let (mut args, mut word, mut depth) = (Vec::new(), String::new(), 0i32);
    let mut chars = rest.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            _ => {}
        }
        if depth <= 0 && c == ':' && chars.peek().is_none_or(|(_, n)| n.is_whitespace()) {
            if !word.is_empty() {
                args.push(std::mem::take(&mut word));
            }
            return (args, rest[i + 1..].trim().to_string());
        }
        if depth <= 0 && c.is_whitespace() {
            if !word.is_empty() {
                args.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    // No colon: the help is all description.
    (Vec::new(), help.to_string())
}

/// Quote a word if it needs it, for writing configs and echoing commands.
pub fn quote(word: &str) -> String {
    if word.is_empty() || word.contains(|c: char| c.is_whitespace() || c == ';' || c == '"') {
        format!("\"{}\"", word.replace('"', "'"))
    } else {
        word.to_string()
    }
}

/// Run queued lines (until a `wait`), in an exclusive system.
pub fn run_queue(world: &mut World) {
    let mut budget = 1024;
    loop {
        let next = {
            let mut c = world.resource_mut::<Console>();
            if c.wait > 0 {
                c.wait -= 1;
                return;
            }
            c.queue.pop_front()
        };
        let Some(line) = next else { return };
        let commands = parse(&line);
        for (i, words) in commands.iter().enumerate() {
            execute(world, words, 0);
            budget -= 1;
            if world.resource::<Console>().wait > 0 || budget == 0 {
                // The rest of this line runs after the wait.
                let rest: Vec<String> = commands[i + 1..]
                    .iter()
                    .map(|w| w.iter().map(|x| quote(x)).collect::<Vec<_>>().join(" "))
                    .collect();
                if !rest.is_empty() {
                    world.resource_mut::<Console>().queue.push_front(rest.join("; "));
                }
                return;
            }
        }
    }
}

/// Run one command (its words), with aliases expanded.
pub fn execute(world: &mut World, words: &[String], depth: usize) {
    let Some(name) = words.first().map(|w| w.to_lowercase()) else {
        return;
    };
    let args = &words[1..];
    let (alias, command, cvar) = {
        let c = world.resource::<Console>();
        (
            c.aliases.get(&name).cloned(),
            c.command(&name).cloned(),
            c.cvar(&name).cloned(),
        )
    };
    if let Some(body) = alias {
        if depth >= MAX_ALIAS_DEPTH {
            world
                .resource_mut::<Console>()
                .print(Level::Error, format!("{name}: alias loop"));
            return;
        }
        for w in parse(&body) {
            execute(world, &w, depth + 1);
        }
        return;
    }
    let result = if let Some(cmd) = command {
        if SERVER_COMMANDS.contains(&name.as_str())
            && world.get_resource::<crate::core::NetRole>() == Some(&crate::core::NetRole::Client)
        {
            Err(format!(
                "Can't use server command {name} from a client: only the server operator can run it."
            ))
        } else if CHEATS.contains(&name.as_str()) && !cheats_allowed(world) {
            // Source's words.
            Err(format!(
                "Can't use cheat command {name} in multiplayer, unless the server has sv_cheats set to 1."
            ))
        } else {
            (cmd.run)(world, args)
        }
    } else if let Some(cvar) = cvar {
        if args.is_empty() {
            let value = (cvar.get)(world).unwrap_or_default();
            let changed = if value != cvar.default {
                format!(" (default \"{}\")", cvar.default)
            } else {
                String::new()
            };
            Ok(Some(format!(
                "\"{}\" = \"{}\"{changed}\n - {}",
                cvar.name, value, cvar.help
            )))
        } else if cvar.scope != CvarScope::Local
            && world.get_resource::<crate::core::NetRole>() == Some(&crate::core::NetRole::Client)
        {
            // The server's (sent to us, `net::cvars`), Source's words.
            Err(format!("Can't change replicated ConVar {} from console of client, only server operator can change its value", cvar.name))
        } else {
            (cvar.set)(world, &args.join(" ")).map(|_| {
                world.resource_mut::<Console>().dirty = true;
                None
            })
        }
    } else {
        Err(format!("Unknown command \"{name}\""))
    };
    let mut c = world.resource_mut::<Console>();
    match result {
        Ok(Some(out)) => c.print(Level::Info, out),
        Ok(None) => {}
        Err(e) => c.print(Level::Error, e),
    }
}

/// Register cvars and commands on the app.
pub trait ConsoleAppExt {
    fn console_cvar(
        &mut self,
        name: &str,
        help: &str,
        default: &str,
        get: impl Fn(&mut World) -> Option<String> + Send + Sync + 'static,
        set: impl Fn(&mut World, &str) -> Result<(), String> + Send + Sync + 'static,
    ) -> &mut Self;
    fn console_command(
        &mut self,
        name: &str,
        help: &str,
        run: impl Fn(&mut World, &[String]) -> Result<Option<String>, String> + Send + Sync + 'static,
    ) -> &mut Self;
}

impl ConsoleAppExt for App {
    fn console_cvar(
        &mut self,
        name: &str,
        help: &str,
        default: &str,
        get: impl Fn(&mut World) -> Option<String> + Send + Sync + 'static,
        set: impl Fn(&mut World, &str) -> Result<(), String> + Send + Sync + 'static,
    ) -> &mut Self {
        self.init_resource::<Console>();
        self.world_mut().resource_mut::<Console>().add_cvar(Cvar {
            name: name.into(),
            help: help.into(),
            default: default.into(),
            values: if default == "0" || default == "1" {
                vec!["0".into(), "1".into()]
            } else {
                Vec::new()
            },
            archive: false,
            range: None,
            integer: false,
            scope: CvarScope::of(name),
            get: Arc::new(get),
            set: Arc::new(set),
        });
        self
    }

    fn console_command(
        &mut self,
        name: &str,
        help: &str,
        run: impl Fn(&mut World, &[String]) -> Result<Option<String>, String> + Send + Sync + 'static,
    ) -> &mut Self {
        self.init_resource::<Console>();
        self.world_mut().resource_mut::<Console>().add_command(Command {
            name: name.into(),
            help: help.into(),
            run: Arc::new(run),
            complete: None,
        });
        self
    }
}

/// A cvar backed by a field of a resource, parsed and printed with
/// `FromStr`/`Display`.
pub fn resource_cvar<R, T>(app: &mut App, name: &str, help: &str, field: fn(&mut R) -> &mut T)
where
    R: Resource + bevy::ecs::component::Component<Mutability = bevy::ecs::component::Mutable>,
    T: std::str::FromStr + std::fmt::Display + Send + Sync + 'static,
{
    let default = app
        .world_mut()
        .get_resource_mut::<R>()
        .map(|mut r| field(&mut r).to_string())
        .unwrap_or_default();
    app.console_cvar(
        name,
        help,
        &default,
        move |w| {
            // One accessor serves both; reading doesn't count as a change.
            w.get_resource_mut::<R>()
                .map(|mut r| field(r.bypass_change_detection()).to_string())
        },
        move |w, v| {
            let mut r = w.get_resource_mut::<R>().ok_or("not available")?;
            *field(&mut r) = v.trim().parse().map_err(|_| format!("bad value \"{v}\""))?;
            Ok(())
        },
    );
    // Whole numbers only: the type reads "1" but not "0.5". A fractional
    // one isn't on/off even when its default is 0 or 1.
    let integer = "1".parse::<T>().is_ok() && "0.5".parse::<T>().is_err();
    let mut console = app.world_mut().resource_mut::<Console>();
    if let Some(c) = console.cvars.get_mut(&name.to_lowercase()) {
        c.integer = integer;
        if "0.5".parse::<T>().is_ok() {
            c.values.clear();
        }
    }
}

/// Commands that change the simulation for the one who runs them
/// (Source's FCVAR_CHEAT ones): in a network game (server or client) only
/// with `sv_cheats 1` on the server (replicated). Single player always
/// has them.
pub const CHEATS: &[&str] = &[
    "noclip",
    "god",
    "setpos",
    "setang",
    "give",
    "impulse",
    "ent_fire",
    "mashup_hurtme",
    "mashup_sethealth",
    "mashup_setarmor",
    "mashup_setmoney",
];

/// Commands that change the server's game (bots, the map): a network
/// client can't run them (its world isn't the game's; the server's
/// operator runs them on the server's console).
pub const SERVER_COMMANDS: &[&str] = &[
    "bot_add",
    "bot_add_t",
    "bot_add_ct",
    "bot_kick",
    "bot_give",
    "bot_goto",
    "changelevel",
    "mp_restartgame",
    "kick",
    "kickid",
];

/// `sv_cheats`.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cheats(pub u8);

/// Whether cheat commands work here: single player, or `sv_cheats 1`.
pub fn cheats_allowed(world: &World) -> bool {
    world
        .get_resource::<crate::core::NetRole>()
        .is_none_or(|r| *r == crate::core::NetRole::Standalone)
        || world.get_resource::<Cheats>().is_some_and(|c| c.0 != 0)
}

/// The core commands.
pub struct ConsolePlugin;

impl Plugin for ConsolePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Console>()
            .init_resource::<Cheats>()
            .add_systems(PreUpdate, run_queue);
        builtins(app);
        resource_cvar::<Cheats, u8>(
            app,
            "sv_cheats",
            "1: cheat commands (noclip, god, setpos, give, ...) work in a network game (single player always has them).",
            |c| &mut c.0,
        );
    }
}

fn console(w: &mut World) -> Mut<'_, Console> {
    w.resource_mut::<Console>()
}

fn builtins(app: &mut App) {
    app.console_command("echo", "Print text.", |_, a| Ok(Some(a.join(" "))))
        .console_command("clear", "Clear the console output.", |w, _| {
            console(w).output.clear();
            Ok(None)
        })
        .console_command("help", "help <name>: what a command or cvar does.", |w, a| {
            let c = w.resource::<Console>();
            let Some(name) = a.first() else {
                return Ok(Some(
                    "Type a command or cvar name: suggestions and the arguments' help show as you type.\n\
                     Up/Down pick a suggestion (or browse history with an empty line), Enter or Tab takes it, Esc hides them;\n\
                     Tab completes and cycles, Ctrl+R searches history, Ctrl+L clears, Ctrl+V pastes.\n\
                     find <text> searches names and help; cvarlist / cmdlist list them; differences shows changed cvars."
                        .into(),
                ));
            };
            if let Some(cmd) = c.command(name) {
                return Ok(Some(format!("{}: {}", cmd.name, cmd.help)));
            }
            if let Some(v) = c.cvar(name) {
                let range = v.range.map_or(String::new(), |(a, b)| format!(", range {a} to {b}"));
                return Ok(Some(format!("{} (cvar, default \"{}\"{range}): {}", v.name, v.default, v.help)));
            }
            if let Some(body) = c.aliases.get(&name.to_lowercase()) {
                return Ok(Some(format!("{name} is an alias for: {body}")));
            }
            Err(format!("no command or cvar \"{name}\""))
        })
        .console_command("find", "find <text>: cvars and commands whose name or help mention it.", |w, a| {
            let text = a.join(" ").to_lowercase();
            let c = w.resource::<Console>();
            let mut out: Vec<String> = c
                .cvars()
                .filter(|v| v.name.to_lowercase().contains(&text) || v.help.to_lowercase().contains(&text))
                .map(|v| format!("{:<28} cvar  {}", v.name, v.help))
                .chain(
                    c.commands()
                        .filter(|v| v.name.to_lowercase().contains(&text) || v.help.to_lowercase().contains(&text))
                        .map(|v| format!("{:<28} cmd   {}", v.name, v.help)),
                )
                .collect();
            out.sort();
            Ok(Some(if out.is_empty() { format!("nothing matches \"{text}\"") } else { out.join("\n") }))
        })
        .console_command("cvarlist", "cvarlist [prefix]: cvars with their values.", |w, a| {
            let prefix = a.first().map(|p| p.to_lowercase()).unwrap_or_default();
            let cvars: Vec<Cvar> = w
                .resource::<Console>()
                .cvars()
                .filter(|v| v.name.to_lowercase().starts_with(&prefix))
                .cloned()
                .collect();
            let lines: Vec<String> = cvars
                .iter()
                .map(|v| {
                    let value = (v.get)(w).unwrap_or_default();
                    let mark = if value != v.default { "*" } else { " " };
                    format!("{mark} {:<28} = {:<10} {}", v.name, value, v.help)
                })
                .collect();
            Ok(Some(format!("{}\n{} cvars (* changed from default)", lines.join("\n"), lines.len())))
        })
        .console_command("cmdlist", "cmdlist [prefix]: commands.", |w, a| {
            let prefix = a.first().map(|p| p.to_lowercase()).unwrap_or_default();
            let lines: Vec<String> = w
                .resource::<Console>()
                .commands()
                .filter(|v| v.name.to_lowercase().starts_with(&prefix))
                .map(|v| format!("{:<28} {}", v.name, v.help))
                .collect();
            Ok(Some(lines.join("\n")))
        })
        .console_command("differences", "Cvars changed from their defaults.", |w, _| {
            let cvars: Vec<Cvar> = w.resource::<Console>().cvars().cloned().collect();
            let lines: Vec<String> = cvars
                .iter()
                .filter_map(|v| {
                    let value = (v.get)(w)?;
                    (value != v.default).then(|| format!("{} \"{}\" (default \"{}\")", v.name, value, v.default))
                })
                .collect();
            Ok(Some(if lines.is_empty() { "no cvars changed".into() } else { lines.join("\n") }))
        })
        .console_command("reset", "reset <cvar>: back to its default.", |w, a| {
            let name = a.first().ok_or("reset <cvar>")?;
            let v = w.resource::<Console>().cvar(name).cloned().ok_or(format!("no cvar \"{name}\""))?;
            (v.set)(w, &v.default)?;
            Ok(Some(format!("{} = \"{}\"", v.name, v.default)))
        })
        .console_command("resetall", "Every cvar back to its default.", |w, _| {
            let cvars: Vec<Cvar> = w.resource::<Console>().cvars().cloned().collect();
            let mut n = 0;
            for v in &cvars {
                if (v.get)(w).is_some_and(|x| x != v.default) {
                    (v.set)(w, &v.default)?;
                    n += 1;
                }
            }
            Ok(Some(format!("{n} cvars reset")))
        })
        .console_command("alias", "alias <name> \"<commands>\": a new command; alias alone lists them.", |w, a| {
            let mut c = console(w);
            match a {
                [] => Ok(Some(
                    c.aliases.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n"),
                )),
                [name] => Ok(c.aliases.get(&name.to_lowercase()).map(|v| format!("{name}: {v}"))),
                [name, rest @ ..] => {
                    c.aliases.insert(name.to_lowercase(), rest.join(" "));
                    c.dirty = true;
                    Ok(None)
                }
            }
        })
        .console_command("toggle", "toggle <cvar> [a b ...]: flip 0/1, or cycle through the values given.", |w, a| {
            let name = a.first().ok_or("toggle <cvar> [values]")?;
            let v = w.resource::<Console>().cvar(name).cloned().ok_or(format!("no cvar \"{name}\""))?;
            let now = (v.get)(w).unwrap_or_default();
            let cycle: Vec<String> = if a.len() > 1 { a[1..].to_vec() } else { vec!["0".into(), "1".into()] };
            let i = cycle.iter().position(|x| x == &now).map_or(0, |i| (i + 1) % cycle.len());
            (v.set)(w, &cycle[i])?;
            Ok(Some(format!("{} = \"{}\"", v.name, cycle[i])))
        })
        .console_command("incrementvar", "incrementvar <cvar> <min> <max> <step>: add step, wrapping.", |w, a| {
            let [name, min, max, step] = a else {
                return Err("incrementvar <cvar> <min> <max> <step>".into());
            };
            let num = |s: &String| s.parse::<f64>().map_err(|_| format!("bad number \"{s}\""));
            let (min, max, step) = (num(min)?, num(max)?, num(step)?);
            let v = w.resource::<Console>().cvar(name).cloned().ok_or(format!("no cvar \"{name}\""))?;
            let now: f64 = (v.get)(w).and_then(|x| x.parse().ok()).unwrap_or(min);
            let mut next = now + step;
            if next > max + 1e-9 {
                next = min;
            } else if next < min - 1e-9 {
                next = max;
            }
            let text = if next.fract() == 0.0 { format!("{next:.0}") } else { format!("{next}") };
            (v.set)(w, &text)?;
            Ok(Some(format!("{} = \"{text}\"", v.name)))
        })
        .console_command("wait", "wait [frames]: pause the rest of the queue (default 1 frame).", |w, a| {
            let frames = a.first().and_then(|f| f.parse().ok()).unwrap_or(1);
            console(w).wait = frames;
            Ok(None)
        })
        .console_command("watch", "watch <cvar>: show its value on screen (unwatch to remove).", |w, a| {
            let name = a.first().ok_or("watch <cvar>")?.to_lowercase();
            let mut c = console(w);
            if !c.watches.contains(&name) {
                c.watches.push(name);
            }
            Ok(None)
        })
        .console_command("unwatch", "unwatch [cvar]: stop showing it (all with no name).", |w, a| {
            let mut c = console(w);
            match a.first() {
                Some(n) => c.watches.retain(|x| x != &n.to_lowercase()),
                None => c.watches.clear(),
            }
            Ok(None)
        })
        .console_command("exec", "exec <file>: run a config file (cfg folder; .cfg optional).", |w, a| {
            let name = a.first().ok_or("exec <file>")?;
            let text = read_cfg(name).ok_or(format!("couldn't exec {name}"))?;
            run_cfg(w, &text);
            Ok(None)
        })
        .console_command("execifexists", "execifexists <file>: exec, silently skipped if missing.", |w, a| {
            if let Some(text) = a.first().and_then(|n| read_cfg(n)) {
                run_cfg(w, &text);
            }
            Ok(None)
        })
        .console_command("bind", "bind <key> [command]: run a command on a key (+action: held).", |w, a| {
            let mut c = console(w);
            match a {
                [] => Err("bind <key> [command]".into()),
                [key] => Ok(Some(match c.binds.get(&key.to_lowercase()) {
                    Some(cmd) => format!("\"{key}\" = \"{cmd}\""),
                    None => format!("\"{key}\" is not bound"),
                })),
                [key, rest @ ..] => {
                    c.binds.insert(key.to_lowercase(), rest.join(" "));
                    c.dirty = true;
                    Ok(None)
                }
            }
        })
        .console_command("unbind", "unbind <key>", |w, a| {
            let key = a.first().ok_or("unbind <key>")?.to_lowercase();
            let mut c = console(w);
            c.binds.remove(&key);
            c.dirty = true;
            Ok(None)
        })
        .console_command("unbindall", "Remove every bind.", |w, _| {
            let mut c = console(w);
            c.binds.clear();
            c.dirty = true;
            Ok(None)
        })
        .console_command("bindlist", "Every bind.", |w, _| {
            let c = w.resource::<Console>();
            Ok(Some(c.binds.iter().map(|(k, v)| format!("{k:<12} \"{v}\"")).collect::<Vec<_>>().join("\n")))
        })
        .console_command("host_writeconfig", "host_writeconfig [file]: save binds, aliases and changed cvars (config.cfg).", |w, a| {
            let name = a.first().map_or("config.cfg", |s| s.as_str());
            let text = config_text(w);
            let path = cfg_dir().ok_or("no config folder")?.join(with_cfg(name));
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, text).map_err(|e| e.to_string())?;
            console(w).dirty = false;
            Ok(Some(format!("wrote {}", path.display())))
        });
}

/// Where configs live: the user's data folder (Linux ~/.local/share/mashup/cfg,
/// Windows %APPDATA%\\mashup\\cfg).
pub fn cfg_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(std::path::PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))?;
    Some(base.join("mashup").join("cfg"))
}

fn with_cfg(name: &str) -> String {
    if name.to_lowercase().ends_with(".cfg") {
        name.to_string()
    } else {
        format!("{name}.cfg")
    }
}

/// A config file's text: an absolute or relative path as given, else from
/// the cfg folder.
pub fn read_cfg(name: &str) -> Option<String> {
    let given = std::path::Path::new(name);
    let candidates = [
        given.to_path_buf(),
        given.with_extension("cfg"),
        cfg_dir()?.join(with_cfg(name)),
    ];
    candidates.iter().find_map(|p| std::fs::read_to_string(p).ok())
}

/// Run a config's lines before anything else queued, in file order.
fn run_cfg(w: &mut World, text: &str) {
    let mut c = console(w);
    for line in text.lines().rev() {
        if !line.trim().is_empty() {
            c.queue.push_front(line.to_string());
        }
    }
}

/// Binds, aliases and changed cvars, as a config file.
pub fn config_text(w: &mut World) -> String {
    // The second line tells the client every key's bind is in here
    // (`client::binds::CONFIG_MARK`).
    let mut out = String::from("// Written by mashup (host_writeconfig).\n// binds: all\n");
    let (binds, aliases, cvars) = {
        let c = w.resource::<Console>();
        (
            c.binds.clone(),
            c.aliases.clone(),
            c.cvars().cloned().collect::<Vec<_>>(),
        )
    };
    out += "unbindall\n";
    for (k, v) in binds {
        out += &format!("bind {} {}\n", quote(&k), quote(&v));
    }
    for (k, v) in aliases {
        out += &format!("alias {} {}\n", quote(&k), quote(&v));
    }
    for v in cvars.into_iter().filter(|v| v.archive) {
        if let Some(value) = (v.get)(w)
            && value != v.default
        {
            out += &format!("{} {}\n", v.name, quote(&value));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cvar_scopes_follow_the_server_prefixes() {
        assert_eq!(CvarScope::of("sv_gravity"), CvarScope::Replicated);
        assert_eq!(CvarScope::of("mp_friendlyfire"), CvarScope::Replicated);
        assert_eq!(CvarScope::of("bot_stop"), CvarScope::Server);
        assert_eq!(CvarScope::of("sv_password"), CvarScope::Server);
        assert_eq!(CvarScope::of("cl_crosshairscale"), CvarScope::Local);
        assert_eq!(CvarScope::of("sensitivity"), CvarScope::Local);
    }

    #[test]
    fn usage_and_argument_positions() {
        let (args, text) = usage("setpos", "setpos <x> <y> <z>: move there (CS:S units).");
        assert_eq!(args, ["<x>", "<y>", "<z>"]);
        assert_eq!(text, "move there (CS:S units).");
        // Brackets keep their spaces and colons.
        let (args, text) = usage(
            "mashup_hurtme",
            "mashup_hurtme <head|chest> [amount=100] [from yaw, degrees: 0 = front]: a hit on yourself.",
        );
        assert_eq!(args, ["<head|chest>", "[amount=100]", "[from yaw, degrees: 0 = front]"]);
        assert_eq!(text, "a hit on yourself.");
        // Help without a usage is all description.
        assert_eq!(usage("god", "Toggle taking no damage."), (vec![], "Toggle taking no damage.".to_string()));
        assert_eq!(usage("give", "Give the local player a weapon by ID (e.g. give x).").0, Vec::<String>::new());
        // A name that only starts the help isn't a usage.
        assert_eq!(usage("bot", "bot_add [team]: add a bot.").0, Vec::<String>::new());
        assert_eq!(arg_position("setpos"), None);
        assert_eq!(arg_position("setpos "), Some(0));
        assert_eq!(arg_position("setpos 1"), Some(0));
        assert_eq!(arg_position("setpos 1 2 "), Some(2));
        assert_eq!(last_command_start("echo a; sv_gr"), 8);
        assert_eq!(last_command_start("bind x \"+jump; say\" "), 0);
        assert_eq!(last_command_start("noclip"), 0);
    }

    #[test]
    fn parsing() {
        assert_eq!(parse("a b; c \"d e\""), vec![vec!["a", "b"], vec!["c", "d e"]]);
        assert_eq!(
            parse("bind x \"+jump; say hi\" // note"),
            vec![vec!["bind", "x", "+jump; say hi"]]
        );
        assert_eq!(parse("  ;; "), Vec::<Vec<String>>::new());
        assert_eq!(parse("echo \"\""), vec![vec!["echo", ""]]);
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(ConsolePlugin);
        #[derive(Resource)]
        struct Speed(f32);
        app.insert_resource(Speed(250.0));
        resource_cvar::<Speed, f32>(&mut app, "sv_speed", "a speed", |s| &mut s.0);
        app
    }

    fn run(app: &mut App, line: &str) -> Vec<String> {
        let start = app.world().resource::<Console>().output.len();
        app.world_mut().resource_mut::<Console>().submit(line);
        app.update();
        app.world().resource::<Console>().output[start..]
            .iter()
            .map(|l| l.text.clone())
            .collect()
    }

    #[test]
    fn cvars_aliases_and_builtins() {
        let mut app = app();
        assert!(run(&mut app, "sv_speed")[0].contains("\"250\""));
        run(&mut app, "sv_speed 300");
        assert!(run(&mut app, "differences")[0].contains("sv_speed \"300\""));
        run(&mut app, "alias fast \"sv_speed 500; echo zoom\"");
        assert_eq!(run(&mut app, "fast"), vec!["zoom"]);
        assert!(run(&mut app, "sv_speed")[0].contains("\"500\""));
        run(&mut app, "reset sv_speed");
        assert!(run(&mut app, "sv_speed")[0].contains("\"250\""));
        run(&mut app, "toggle sv_speed 250 320");
        assert!(run(&mut app, "sv_speed")[0].contains("\"320\""));
        run(&mut app, "incrementvar sv_speed 250 330 10");
        assert!(run(&mut app, "sv_speed")[0].contains("\"330\""));
        run(&mut app, "incrementvar sv_speed 250 330 10");
        assert!(run(&mut app, "sv_speed")[0].contains("\"250\""), "wraps");
        assert!(run(&mut app, "nope")[0].contains("Unknown command"));
        assert!(run(&mut app, "find speed").iter().any(|l| l.contains("sv_speed")));
    }

    #[test]
    fn config_round_trip() {
        let mut first = app();
        // Only archived cvars are saved.
        first.world_mut().resource_mut::<Console>().archive("sv_speed");
        run(
            &mut first,
            "bind space \"+jump\"; bind mwheeldown +jump; alias hop \"echo hi; wait\"; sv_speed 320",
        );
        let text = config_text(first.world_mut());
        assert!(text.contains("bind space +jump"), "{text}");
        assert!(text.contains("alias hop \"echo hi; wait\""), "{text}");
        assert!(text.contains("sv_speed 320"), "{text}");
        // A fresh console running the file ends up the same.
        let mut other = app();
        run_cfg(other.world_mut(), &text);
        other.update();
        let c = other.world().resource::<Console>();
        assert_eq!(c.binds.get("space").map(String::as_str), Some("+jump"));
        assert_eq!(c.aliases.get("hop").map(String::as_str), Some("echo hi; wait"));
        assert!(run(&mut other, "sv_speed")[0].contains("\"320\""));
        // Not archived: not saved.
        other
            .world_mut()
            .resource_mut::<Console>()
            .cvars
            .get_mut("sv_speed")
            .unwrap()
            .archive = false;
        assert!(!config_text(other.world_mut()).contains("sv_speed"));
    }

    #[test]
    fn wait_pauses_the_queue() {
        let mut app = app();
        assert_eq!(run(&mut app, "echo a; wait 2; echo b"), vec!["a"]);
        // Two frames of waiting, then the rest of the line runs.
        let mut frames = 0;
        while !app.world().resource::<Console>().output.iter().any(|l| l.text == "b") {
            app.update();
            frames += 1;
            assert!(frames <= 3, "b never ran");
        }
        assert_eq!(frames, 3);
    }
}
