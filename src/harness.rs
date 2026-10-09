//! Headless simulation for tests and tools: no window, no rendering, time
//! advanced by exact fixed ticks. Scenario tests in `tests/` build on this.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use avian3d::prelude::*;
use bevy::{prelude::*, time::TimeUpdateStrategy};

use crate::{
    DEFAULT_TICK_HZ, SimPlugins,
    character::character_bundle,
    core::{Intent, LocalPlayer, MovementState, SimTick, Team, Velocity},
    net::memory::{Link, LinkConditions},
    slots::set_movement,
};

pub struct Sim {
    pub app: App,
}

/// The simulation with no window or rendering: what `Sim` runs (stepped by
/// hand).
pub fn add_headless(app: &mut App) {
    add_headless_with(app, MinimalPlugins.build());
}

/// The same with `minimal` (`MinimalPlugins`, e.g. with a run loop that
/// waits between frames: the dedicated server).
pub fn add_headless_with(app: &mut App, minimal: bevy::app::PluginGroupBuilder) {
    app.add_plugins((
        minimal,
        TransformPlugin,
        // avian watches mesh assets even when no collider uses them.
        AssetPlugin::default(),
        bevy::mesh::MeshPlugin,
        // Interpolation eases rendering between ticks; headless, it
        // would make `Transform` show the previous tick.
        PhysicsPlugins::default()
            .with_collision_hooks::<crate::map::MapCollisionHooks>()
            .build()
            .disable::<PhysicsInterpolationPlugin>(),
        SimPlugins,
    ));
}

/// `Sim::pad`'s resource.
#[derive(Resource)]
struct Padding(#[allow(dead_code)] usize);

/// `MASHUP_TEST_PAD`, or 0.
fn pad_from_env() -> usize {
    std::env::var("MASHUP_TEST_PAD").ok().and_then(|v| v.parse().ok()).unwrap_or(0)
}

impl Sim {
    /// A headless app with the simulation plugins plus `plugins` (a map,
    /// and any game plugins whose implementations the test uses).
    pub fn new<M>(plugins: impl bevy::app::Plugins<M>) -> Self {
        let mut app = App::new();
        add_headless(&mut app);
        app.add_plugins(plugins);
        Self::start(app)
    }

    /// Like `new`, with the plugins added by `setup` (a `NetSim` builds
    /// several apps from one setup).
    pub fn with(setup: impl FnOnce(&mut App)) -> Self {
        let mut app = App::new();
        add_headless(&mut app);
        setup(&mut app);
        Self::start(app)
    }

    fn start(mut app: App) -> Self {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / DEFAULT_TICK_HZ,
        )));
        app.finish();
        app.cleanup();
        // Startup, and one tick so the physics spatial index contains the map.
        app.update();
        let mut sim = Self { app };
        sim.pad(pad_from_env());
        sim.ticks(1);
        sim
    }

    /// Spawn `n` empty entities and a resource: every entity spawned
    /// after gets another id (Bevy 0.19 resources are entities too).
    /// Nothing may depend on entity ids; `MASHUP_TEST_PAD=<n>` pads
    /// every `Sim` this way, to check a test's result doesn't change.
    pub fn pad(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        let world = self.app.world_mut();
        for _ in 0..n {
            world.spawn_empty();
        }
        world.insert_resource(Padding(n));
    }

    /// Run fixed ticks of `secs` from now on (e.g. a game's own tick).
    pub fn set_tick_interval(&mut self, secs: f64) {
        let step = Duration::from_secs_f64(secs);
        self.app.insert_resource(Time::<Fixed>::from_duration(step));
        self.app.insert_resource(TimeUpdateStrategy::ManualDuration(step));
    }

    pub fn tick_hz(&self) -> f64 {
        self.app
            .world()
            .resource::<Time<Fixed>>()
            .timestep()
            .as_secs_f64()
            .recip()
    }

    /// Advance exactly `n` fixed ticks.
    pub fn ticks(&mut self, n: u64) {
        let target = self.tick() + n;
        while self.tick() < target {
            self.app.update();
        }
    }

    /// Advance by whole ticks covering `secs` seconds.
    pub fn seconds(&mut self, secs: f64) {
        self.ticks((secs * self.tick_hz()).round() as u64);
    }

    pub fn tick(&self) -> u64 {
        self.app.world().resource::<SimTick>().0
    }

    pub fn spawn_character(&mut self, position: Vec3, movement: &'static str) -> Entity {
        let world = self.app.world_mut();
        let entity = world
            .spawn(character_bundle(Transform::from_translation(position), Team(0)))
            .id();
        set_movement(entity, movement).apply(world);
        entity
    }

    pub fn set_movement(&mut self, entity: Entity, movement: &'static str) {
        set_movement(entity, movement).apply(self.app.world_mut());
    }

    pub fn intent(&mut self, entity: Entity) -> Mut<'_, Intent> {
        self.app
            .world_mut()
            .get_mut::<Intent>(entity)
            .expect("entity has no Intent")
    }

    pub fn position(&self, entity: Entity) -> Vec3 {
        self.app
            .world()
            .get::<Transform>(entity)
            .expect("no Transform")
            .translation
    }

    pub fn velocity(&self, entity: Entity) -> Vec3 {
        self.app.world().get::<Velocity>(entity).expect("no Velocity").0
    }

    pub fn state(&self, entity: Entity) -> MovementState {
        self.app
            .world()
            .get::<MovementState>(entity)
            .expect("no MovementState")
            .clone()
    }
}

type Setup = Arc<dyn Fn(&mut App) + Send + Sync>;

/// A server and clients in one process, each a headless `Sim`, joined by
/// an in-memory link (`net::memory`) with seeded latency, jitter and loss.
/// Stepped together one tick at a time: the server, then each client.
pub struct NetSim {
    pub server: Sim,
    pub clients: Vec<Sim>,
    pub link: Arc<Mutex<Link>>,
    setup: Setup,
    /// Real time per `step` (a frame everywhere): a tick unless
    /// `set_frame` says otherwise.
    frame: Duration,
}

impl NetSim {
    /// A server running what `setup` adds (a map, game plugins) with
    /// `clients` clients connected (not yet joined: step until they are,
    /// e.g. `until_joined`). Client ids are 1, 2, ...
    pub fn new(
        conditions: LinkConditions,
        seed: u64,
        clients: usize,
        setup: impl Fn(&mut App) + Send + Sync + 'static,
    ) -> Self {
        let setup: Setup = Arc::new(setup);
        let link = Link::new(conditions, seed);
        let mut server = Sim::with(|app| {
            app.add_plugins(crate::net::NetPlugin);
            setup(app);
        });
        // Room for any test's players (`a_full_server_refuses` sets fewer).
        server.app.world_mut().resource_mut::<crate::net::NetSettings>().maxplayers = 32;
        crate::net::memory::serve(server.app.world_mut(), link.clone()).expect("serve");
        let mut sim = Self {
            server,
            clients: Vec::new(),
            link,
            setup,
            frame: Duration::from_secs_f64(1.0 / DEFAULT_TICK_HZ),
        };
        for _ in 0..clients {
            sim.add_client(|_| {});
        }
        sim
    }

    /// Connect another client, after `before` changes its world (e.g. its
    /// `net::NetVersion`). Returns its index in `clients`.
    pub fn add_client(&mut self, before: impl FnOnce(&mut World)) -> usize {
        let setup = self.setup.clone();
        let mut client = Sim::with(|app| {
            app.add_plugins(crate::net::NetPlugin);
            setup(app);
        });
        client.app.insert_resource(TimeUpdateStrategy::ManualDuration(self.frame));
        before(client.app.world_mut());
        let id = self.clients.len() as u64 + 1;
        crate::net::memory::join(client.app.world_mut(), self.link.clone(), id).expect("join");
        self.clients.push(client);
        self.clients.len() - 1
    }

    /// Client `i`'s id (`NetCharacter::owner` of its character).
    pub fn client_id(&self, i: usize) -> u64 {
        i as u64 + 1
    }

    /// Every app's frame from now on (s): e.g. 1/240 for frames between
    /// ticks, as a game draws them. `step` then advances by that.
    pub fn set_frame(&mut self, secs: f64) {
        self.frame = Duration::from_secs_f64(secs);
        for sim in std::iter::once(&mut self.server).chain(&mut self.clients) {
            sim.app.insert_resource(TimeUpdateStrategy::ManualDuration(self.frame));
        }
    }

    /// One frame everywhere (a tick unless `set_frame` changed it): the
    /// link's clock, the server, every client.
    pub fn step(&mut self) {
        let dt = self.frame;
        self.link.lock().unwrap().advance(dt);
        self.server.app.update();
        for c in &mut self.clients {
            c.app.update();
        }
    }

    pub fn ticks(&mut self, n: u64) {
        for _ in 0..n {
            self.step();
        }
    }

    /// Step until `done` holds (true) or `max` ticks pass (false).
    pub fn until(&mut self, max: u64, mut done: impl FnMut(&mut Self) -> bool) -> bool {
        for _ in 0..max {
            if done(self) {
                return true;
            }
            self.step();
        }
        done(self)
    }

    /// Step until every client has joined (its map checked) and sees its
    /// own character; panics after `max` ticks.
    pub fn until_joined(&mut self, max: u64) {
        let ok = self.until(max, |s| {
            (0..s.clients.len()).all(|i| {
                s.clients[i].app.world().contains_resource::<crate::net::client::Joined>() && s.local_player(i).is_some()
            })
        });
        assert!(ok, "clients didn't join within {max} ticks");
    }

    /// Client `i`'s own character (its `LocalPlayer`), in its world.
    pub fn local_player(&mut self, i: usize) -> Option<Entity> {
        let world = self.clients[i].app.world_mut();
        world
            .query_filtered::<Entity, With<LocalPlayer>>()
            .iter(world)
            .next()
    }

    /// The server's character for client `i`.
    pub fn character_of(&mut self, i: usize) -> Option<Entity> {
        let id = self.client_id(i);
        Self::owned_by(&mut self.server, Some(id))
    }

    /// The character owned by `owner` in a sim's world (`None`: a bot).
    pub fn owned_by(sim: &mut Sim, owner: Option<u64>) -> Option<Entity> {
        let world = sim.app.world_mut();
        world
            .query::<(Entity, &crate::net::NetCharacter)>()
            .iter(world)
            .find(|(_, c)| c.owner == owner)
            .map(|(e, _)| e)
    }
}

/// Maps for network tests, built in code from a name: a folder standing
/// for an install (`<install>/maps/<name>.bsp`, any bytes: the file the
/// handshake hashes) and a content cache; `build` makes the map's data
/// (`MapData::file_hash` is set from the file). Map ids are
/// `test:<name>`; `greybox` is the greybox.
#[derive(Resource, Clone)]
pub struct TestMaps {
    pub install: std::path::PathBuf,
    pub cache: Option<std::path::PathBuf>,
    pub build: Arc<dyn Fn(&str) -> crate::map::MapData + Send + Sync>,
}

impl TestMaps {
    /// The file of map `id` in the install folder.
    pub fn file(&self, id: &str) -> std::path::PathBuf {
        self.install
            .join("maps")
            .join(format!("{}.bsp", crate::net::maps::map_name(id)))
    }

    /// Map `id` built, hashed from `file` (else the install's copy).
    pub fn load(&self, id: &str, file: Option<&std::path::Path>) -> Result<crate::map::MapData, String> {
        let path = file.map_or_else(|| self.file(id), std::path::Path::to_path_buf);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut data = (self.build)(crate::net::maps::map_name(id));
        data.name = crate::net::maps::map_name(id).to_string();
        data.file_hash = Some(crate::net::maps::sha256(&bytes));
        Ok(data)
    }
}

/// A test map's tick (the greybox's).
pub const TEST_MAP_TICK: Duration = Duration::from_nanos(15_625_000);

/// Serve and load `maps` in this app as the game does with an install:
/// `net::maps::MapFiles` reads the install folder, and the server's map
/// requests (`NetEvent::LoadMap`) load at once (`load_level`).
pub fn serve_maps(app: &mut App, maps: TestMaps) {
    let read = {
        let maps = maps.clone();
        Arc::new(move |id: &str| std::fs::read(maps.file(id)).ok())
    };
    app.insert_resource(crate::net::maps::MapFiles {
        read,
        cache: maps.cache.clone(),
    })
    .insert_resource(maps)
    .add_systems(Update, load_requested_maps);
}

/// Load map `id` here now (`TestMaps`; `greybox` for the greybox), as the
/// game's `map`/`changelevel` does once its background load is done.
pub fn load_level(world: &mut World, id: &str, file: Option<&std::path::Path>) -> Result<(), String> {
    if id == crate::net::GREYBOX {
        crate::swap_map(world, id, None, TEST_MAP_TICK);
        return Ok(());
    }
    let maps = world.get_resource::<TestMaps>().cloned().ok_or("no TestMaps here")?;
    let data = maps.load(id, file)?;
    crate::swap_map(world, id, Some(data), TEST_MAP_TICK);
    Ok(())
}

/// A server's `changelevel` starting: the map's name changes first (its
/// clients hear "changing level"); `load_level` brings it in.
pub fn begin_level_change(world: &mut World, id: &str) {
    world.insert_resource(crate::map::LoadedMapName(id.to_string()));
}

/// A test client's map loads take this many frames (a slow machine, a big
/// map): `serve_maps` loads what the server asks for that many updates
/// after it asked. 0 (the default without it): at once.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct MapLoadDelay(pub u32);

/// Map loads waiting for their frame (`MapLoadDelay`): frames left, map,
/// file.
#[derive(Resource, Default)]
struct DelayedLoads(Vec<(u32, String, Option<std::path::PathBuf>)>);

fn load_requested_maps(world: &mut World, mut cursor: Local<bevy::ecs::message::MessageCursor<crate::net::NetEvent>>) {
    let events: Vec<crate::net::NetEvent> = cursor
        .read(world.resource::<Messages<crate::net::NetEvent>>())
        .cloned()
        .collect();
    let delay = world.get_resource::<MapLoadDelay>().map_or(0, |d| d.0);
    let mut queue = world.remove_resource::<DelayedLoads>().unwrap_or_default();
    for e in events {
        match e {
            crate::net::NetEvent::LoadMap { map, file } => queue.0.push((delay, map, file)),
            // Leaving drops a load still to come.
            crate::net::NetEvent::Disconnected(_) => queue.0.clear(),
            _ => {}
        }
    }
    let mut due = Vec::new();
    queue.0.retain_mut(|(left, map, file)| {
        if *left == 0 {
            due.push((map.clone(), file.clone()));
            false
        } else {
            *left -= 1;
            true
        }
    });
    world.insert_resource(queue);
    for (map, file) in due {
        if let Err(err) = load_level(world, &map, file.as_deref()) {
            crate::net::disconnect(world, &format!("couldn't load the server's map {map}: {err}"));
        }
    }
}

/// What `emulate_mesh_extraction` saw: each mesh as the render world
/// would hold it, and the meshes it couldn't extract.
#[derive(Resource, Default)]
pub struct MeshExtraction {
    /// The last data extracted per mesh (the render world's copy).
    pub extracted: std::collections::HashMap<AssetId<Mesh>, Mesh>,
    /// Meshes announced as added or modified whose data was already taken:
    /// in a real run, Bevy's "RenderMesh with RenderAssetUsages ==
    /// RENDER_WORLD cannot be extracted: The asset has already been
    /// extracted" error.
    pub failures: Vec<AssetId<Mesh>>,
}

/// Headless stand-in for the render world's mesh extraction, so a test can
/// catch a mesh announced as changed after the render world took its
/// data: each frame (in `Last`, after the asset events go out in
/// `PostUpdate`) a mesh added or modified this frame is extracted as Bevy
/// does: a `RENDER_WORLD`-only one has its data taken from the main world,
/// others are copied. Readers of such meshes look in `MeshExtraction`.
pub fn emulate_mesh_extraction(app: &mut App) {
    app.init_resource::<MeshExtraction>().add_systems(Last, extract_meshes);
}

fn extract_meshes(
    mut events: MessageReader<AssetEvent<Mesh>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mut out: ResMut<MeshExtraction>,
) {
    let Some(mut meshes) = meshes else {
        events.clear();
        return;
    };
    let mut due: Vec<AssetId<Mesh>> = Vec::new();
    for e in events.read() {
        match e {
            AssetEvent::Added { id } | AssetEvent::Modified { id } if !due.contains(id) => due.push(*id),
            _ => {}
        }
    }
    use bevy::asset::RenderAssetUsages;
    for id in due {
        let Some(mesh) = meshes.get_mut_untracked(id) else { continue };
        if !mesh.asset_usage.contains(RenderAssetUsages::RENDER_WORLD) {
            continue;
        }
        if mesh.asset_usage == RenderAssetUsages::RENDER_WORLD {
            match mesh.take_gpu_data() {
                Ok(data) => {
                    out.extracted.insert(id, data);
                }
                Err(_) => out.failures.push(id),
            }
        } else {
            let copy = mesh.clone();
            out.extracted.insert(id, copy);
        }
    }
}
