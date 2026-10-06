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
    pub get: GetFn,
    pub set: SetFn,
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
        (cmd.run)(world, args)
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
}

/// The core commands.
pub struct ConsolePlugin;

impl Plugin for ConsolePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Console>().add_systems(PreUpdate, run_queue);
        builtins(app);
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
                    "Type a command or cvar name. Tab completes, Up/Down for history, Ctrl+R searches it.\n\
                     find <text> searches names and help; cvarlist / cmdlist list them; differences shows changed cvars."
                        .into(),
                ));
            };
            if let Some(cmd) = c.command(name) {
                return Ok(Some(format!("{}: {}", cmd.name, cmd.help)));
            }
            if let Some(v) = c.cvar(name) {
                return Ok(Some(format!("{} (cvar, default \"{}\"): {}", v.name, v.default, v.help)));
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
    let mut out = String::from("// Written by mashup (host_writeconfig).\n");
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
