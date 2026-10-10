# Community maps sweep

Started 2026-10-08. Goal: the user's community CS:S maps (89 in mashup's
content cache: surf_, bhop_, kz_, gg_, mg_) load and play; fix the most
common problems generically (no per-map hacks). Parent plan:
[custom-maps.md](custom-maps.md).

How to check: `cargo run --features dev --bin mapsweep` (docs/OBSERVABILITY.md,
"Sweeping every cached map") writes `target/mapsweep/report.md` (problems
ranked by maps affected, then a row per map), `report.csv`,
`warnings.txt`; `--shots target/playtest/mashup` adds a screenshot from
the first spawn and the frame time there. The maps never enter the repo;
`tests/it/heavy/map_community.rs` tests fixes on cached maps and skips
when a map is absent.

## Results

Sweep of 2026-10-08 (89 maps). Before: 81 loaded, 8 failed (5 panics
in the model reader, 3 load errors), 15 loaded without any warning,
unhandled class or logic complaint. After the fixes below (and main's LZMA
lumps): **89 of 89 load**, 29 fully clean; all 89 render a screenshot from
their first spawn in the playtest build (4 crashed rendering before:
`map::shadows` wrote past a prop's shadow cell). "Warnings" counts load
warnings plus ambient sounds that didn't load; "unhandled entities" are
entities of classes nothing in mashup handles (list below); frame times
are the playtest build at 1280x720, `mashup_perf_log`, vsync off, on the
headless dev box (one view each, so only a hint).

| Map | Before | After | Load s | Warnings before -> after | Unhandled entities | Spawns T/CT | Frame ms at spawn |
|---|---|---|---|---|---|---|---|
| bhop_addict_v2_3xl | ok | ok | 5.0 | 12 -> 1 | 28 | 13/0 | 31.44 |
| bhop_backport_css | ok | ok | 2.0 | 10 -> 0 | 0 | 0/15 | 17.73 |
| bhop_flatzone | ok | ok | 1.7 | 0 -> 0 | 0 | 11/11 | 19.87 |
| bhop_myztek | ok | ok | 9.7 | 1 -> 1 | 0 | 20/20 | 16.94 |
| gg_4mida_arena | ok | ok | 1.5 | 0 -> 0 | 0 | 21/21 | 16.69 |
| gg_beacon | ok | ok | 1.1 | 0 -> 0 | 0 | 16/16 | 16.68 |
| gg_bk_warehouse_v1 | load error | ok | 0.7 | 0 -> 1 | 24 | 12/12 | 16.68 |
| gg_blue_arena_32a | load error | ok | 0.5 | 0 -> 0 | 0 | 18/18 | 16.66 |
| gg_cb_arctic | ok | ok | 0.6 | 0 -> 0 | 1 | 16/16 | 16.68 |
| gg_churches_x_final_fixed | ok | ok | 0.8 | 0 -> 0 | 4 | 16/16 | 16.70 |
| gg_construct_city | ok | ok | 0.9 | 0 -> 0 | 0 | 24/24 | 16.69 |
| gg_deagle7k | ok | ok | 1.1 | 2 -> 2 | 12 | 22/22 | 16.68 |
| gg_desert_paintball | ok | ok | 0.8 | 0 -> 0 | 0 | 44/44 | 17.10 |
| gg_dev_moment_v1 | ok | ok | 0.9 | 0 -> 0 | 24 | 20/20 | 17.76 |
| gg_dinoiceworld | ok | ok | 1.0 | 0 -> 0 | 0 | 16/16 | 16.80 |
| gg_fusion_trx | ok | ok | 0.8 | 0 -> 0 | 1 | 14/14 | 16.71 |
| gg_future | ok | ok | 0.9 | 0 -> 0 | 17 | 28/28 | 16.77 |
| gg_fy_funtimes | ok | ok | 0.9 | 1 -> 0 | 0 | 16/16 | 19.27 |
| gg_fy_tactic_fight | ok | ok | 1.4 | 0 -> 0 | 2 | 16/16 | 16.94 |
| gg_hex | ok | ok | 0.6 | 0 -> 0 | 0 | 12/12 | 16.99 |
| gg_iceworld_l33t | ok | ok | 0.6 | 0 -> 0 | 0 | 12/12 | 16.69 |
| gg_ilu | ok | ok | 1.0 | 1 -> 1 | 1 | 20/20 | 16.67 |
| gg_legendary_fun_v1a | ok | ok | 1.5 | 0 -> 0 | 0 | 16/16 | 23.17 |
| gg_lego_2floor_v2 | ok | ok | 1.9 | 0 -> 0 | 0 | 16/16 | 18.14 |
| gg_lego_mafia_arena | ok | ok | 1.8 | 0 -> 0 | 0 | 16/16 | 19.02 |
| gg_lego_spacetower2 | ok | ok | 1.3 | 0 -> 0 | 0 | 45/45 | 21.71 |
| gg_mario_vs_wario | ok | ok | 3.4 | 0 -> 0 | 0 | 20/20 | 21.64 |
| gg_mario_world_2 | ok | ok | 4.0 | 0 -> 0 | 0 | 28/28 | 16.53 |
| gg_minesweeper | ok | ok | 4.2 | 0 -> 0 | 0 | 16/16 | 18.65 |
| gg_mr_pillar_v1 | ok | ok | 1.9 | 0 -> 0 | 0 | 20/20 | 16.87 |
| gg_nukkon_hdr | ok | ok | 3.0 | 3 -> 3 | 8 | 18/18 | 20.54 |
| gg_nutty | ok | ok | 1.0 | 0 -> 0 | 0 | 16/16 | 18.34 |
| gg_pokemonz | ok | ok | 2.3 | 0 -> 0 | 0 | 51/51 | 17.00 |
| gg_sex_fix | ok | ok | 1.8 | 1 -> 1 | 0 | 30/30 | 17.37 |
| gg_simpsons_arabtoon | ok | ok | 4.0 | 0 -> 0 | 1 | 25/25 | 310.58 |
| gg_simpsons_bam | ok | ok | 9.0 | 1 -> 0 | 1 | 24/24 | 23.70 |
| gg_simpsons_d | ok | ok | 26.0 | 0 -> 0 | 0 | 20/20 | 16.55 |
| gg_simpsons_dusty_2 | ok | ok | 15.1 | 0 -> 0 | 1 | 12/12 | 17.71 |
| gg_simpsons_funfreaks_v2 | ok | ok | 22.3 | 0 -> 0 | 0 | 34/34 | 26.55 |
| gg_spacoid_pg | ok | ok | 19.1 | 0 -> 0 | 27 | 16/16 | 18.05 |
| gg_tbr_water_basin | ok | ok | 13.5 | 0 -> 0 | 0 | 16/15 | 17.32 |
| gg_tip_octal | ok | ok | 6.0 | 1 -> 1 | 1 | 20/20 | 18.17 |
| gg_toondorf | ok | ok | 3.5 | 0 -> 0 | 1 | 21/21 | 17.97 |
| gg_towerwars_v2 | ok | ok | 3.0 | 0 -> 0 | 0 | 16/16 | 18.33 |
| gg_usp_deagle | ok | ok | 1.6 | 0 -> 0 | 1 | 11/12 | 18.65 |
| gg_wolfenstein_3d | ok | ok | 0.9 | 0 -> 0 | 0 | 12/12 | 17.94 |
| kz_11342 | ok | ok | 1.8 | 6 -> 6 | 1 | 0/50 | 17.01 |
| kz_ancient_ruins | ok | ok | 18.9 | 7 -> 0 | 4 | 3/2 | 17.08 |
| kz_bhop_izanami | ok | ok | 17.0 | 1 -> 0 | 261 | 1/1 | 16.67 |
| kz_bhop_sakura | ok | ok | 4.0 | 1 -> 0 | 96 | 0/6 | 16.68 |
| kz_bhop_skodna | ok | ok | 2.4 | 2 -> 2 | 0 | 4/3 | 16.68 |
| kz_hikari_od_nh_v2 | ok | ok | 2.1 | 5 -> 5 | 0 | 5/5 | 16.68 |
| kz_rockb1ock | ok | ok | 0.5 | 11 -> 1 | 3 | 0/32 | 16.70 |
| mg_3k_smash_lego_copter | ok | ok | 1.1 | 6 -> 1 | 73 | 20/20 | 16.62 |
| mg_boatrace_scramble | ok | ok | 1.4 | 1 -> 1 | 115 | 0/20 | 16.57 |
| mg_crazykart_v1_1 | ok | ok | 0.8 | 19 -> 1 | 360 | 32/0 | 16.68 |
| mg_creative_multigames_v8_ns | ok | ok | 4.0 | 62 -> 19 | 121 | 32/32 |  44.76 |
| mg_escape_prison_beta | ok | ok | 0.8 | 11 -> 7 | 30 | 64/0 |  27.77 |
| mg_item_battle_v4b | ok | ok | 7.1 | 0 -> 0 | 196 | 16/16 | 16.67 |
| mg_jacks_multigames_v1 | panic | ok | 5.0 | 0 -> 6 | 262 | 28/27 | 16.68 |
| mg_kommando | panic | ok | 1.9 | 0 -> 8 | 74 | 26/6 |  57.75 |
| mg_lego_course | ok | ok | 1.1 | 1 -> 0 | 19 | 72/0 | 16.68 |
| mg_lego_multigames_v2 | ok | ok | 2.0 | 8 -> 0 | 10 | 33/32 | 16.66 |
| mg_lt_galaxy_v5 | ok | ok | 3.0 | 14 -> 1 | 137 | 32/32 | 16.96 |
| mg_n64_goldeneye_v2 | ok | ok | 3.3 | 7 -> 1 | 44 | 20/20 | 16.71 |
| mg_randomizer_v5 | ok | ok | 3.3 | 14 -> 0 | 91 | 25/25 | 16.68 |
| mg_starwars_v1 | ok | ok | 0.9 | 9 -> 2 | 95 | 16/16 | 16.64 |
| mg_swag_multigames_v1 | panic | ok | 4.6 | 0 -> 2 | 215 | 25/25 | 341.07 |
| mg_wipeout | ok | ok | 3.9 | 1 -> 0 | 2 | 18/18 | 16.67 |
| mg_wipeout2 | load error | ok | 7.0 | 0 -> 1 | 21 | 0/32 | 16.69 |
| surf_apollo | ok | ok | 4.8 | 3 -> 1 | 77 | 32/32 | 16.68 |
| surf_boreas | ok | ok | 6.8 | 20 -> 1 | 57 | 81/81 | 16.68 |
| surf_botanica | ok | ok | 6.2 | 1 -> 0 | 2 | 32/32 | 16.67 |
| surf_demise | ok | ok | 37.8 | 12 -> 10 | 1 | 32/32 | 98.47 |
| surf_halloween_tf2 | ok | ok | 8.2 | 9 -> 8 | 58 | 13/13 | 25.34 |
| surf_happyhands | panic | ok | 4.1 | 0 -> 0 | 60 | 30/30 | 21.14 |
| surf_hellenic | ok | ok | 12.9 | 0 -> 0 | 160 | 32/32 | 24.60 |
| surf_holiday | ok | ok | 15.0 | 4 -> 4 | 0 | 44/33 | 22.47 |
| surf_inferno | ok | ok | 4.6 | 0 -> 0 | 0 | 45/33 | 18.40 |
| surf_jive | ok | ok | 6.3 | 21 -> 5 | 0 | 32/32 | 16.95 |
| surf_kismet | ok | ok | 3.1 | 0 -> 0 | 0 | 50/50 | 18.00 |
| surf_nebula | ok | ok | 2.1 | 0 -> 0 | 30 | 45/33 | 23.52 |
| surf_nsz_fix | ok | ok | 4.5 | 5 -> 3 | 1 | 31/30 | 23.48 |
| surf_sacrifice | ok | ok | 5.8 | 3 -> 1 | 0 | 30/30 | 101.81 |
| surf_sedona | ok | ok | 54.1 | 32 -> 9 | 12 | 32/32 | 16.65 |
| surf_slob | ok | ok | 1.8 | 0 -> 0 | 0 | 32/32 | 17.56 |
| surf_stickybutt_alpha | ok | ok | 0.8 | 0 -> 0 | 3 | 17/17 | 16.60 |
| surf_surreal | panic | ok | 20.7 | 0 -> 9 | 141 | 31/31 | 16.68 |
| surf_threnody | ok | ok | 7.5 | 4 -> 3 | 29 | 32/32 | 16.64 |

## Fixed (2026-10-08)

| Problem | Maps before | Fix |
|---|---|---|
| Panic: vmdl `todo!` on models whose animations are in `.ani` blocks | 4 (mg_jacks_multigames_v1, mg_kommando, mg_swag_multigames_v1, surf_happyhands) | `props::read_mdl`: such models are read without their local animations (our `anim` reads those) |
| Panic: a model whose `.vtx` indexes past its `.vvd` | 1 (surf_surreal) | Checked before converting; skipped with a warning |
| Load error: Latin-1 bytes in the entity lump ("invalid utf-8") | 3 (gg_bk_warehouse_v1, gg_blue_arena_32a, mg_wipeout2) | `bsp::text_lumps_utf8`: invalid bytes become `?` in place |
| LZMA-compressed lumps (lighting, leaves, cubemaps, ... read raw) | 16 (most surf_, kz_ancient_ruins) | On main (`lumps::inflate`, another session) |
| MP3 sounds (music, minigame effects) not decoded | 11 maps, 67 of 80 missing sounds | `wav::decode_any`: MP3 through symphonia |
| Baked cubemaps "not found": the map was renamed after compiling, its pak keeps the old folder | 4 (bhop_addict_v2_3xl, bhop_backport_css, gg_fy_funtimes, kz_rockb1ock) | Looked up by file name under any `materials/maps/*/` |
| Textures in ABGR8888, ARGB8888, BGRX8888, I8, IA88, A8, 565/5551/4444, bluescreen formats | 6 | `material::decode_rgba8`/`decode_full` |
| game_player_equip ignored (players kept the default kit) | 22 | Spawn equipment replaces the starting weapons; Use gives to the activator; strip flag; player_weaponstrip (`weapon::equip`, `core::Equip`) |
| Placed `weapon_*` entities missing | 11 | Loose weapons where the map puts them, put back each round (`MapWeapon`) |
| `!activator` AddOutput gravity / basevelocity (boosters) / health / origin, SetDamageFilter (no-fall filters) | 24 / 13 / 11 / 5 / 3 maps use them | Player keyvalues in `LogicWorld::player_keyvalue`; `core::DamageFilter` |
| Rendering crashed on 4 maps (mg_creative_multigames_v8_ns, mg_escape_prison_beta, mg_kommando, surf_happyhands): a prop triangle beyond its shadow cell indexed past it | 4 | `shadows::silhouette` skips triangles outside the cell |
| Decals kept across rounds (backlog 0) | | `r_cleardecals`; `mashup_round_cleardecals 1` clears at each round start |
| Materials the VMT parser refused: `$detailscale "[9 9 9]"`, WorldVertexTransition without `$basetexture2`, `$basetexturetransform "11"`, a missing `}`, `">=DX90"` blocks, a key twice, `{r g b a}` colours | 32 materials, 10 maps | Read again leniently, the game's way (`material::lenient_vmt`); 1 left (an empty file) |
| Unknown shaders WindowImposter, ShatteredGlass, LightmappedReflective, `Refract_DX90` | 4 | Stand-ins (`material::StandIn`; WindowImposter: its cubemap in the view direction, unlit); specs/cs_source/shaders.md open question 15. Left, on purpose: Screenspace_General (1 map, mg_creative_multigames_v8_ns): a full-screen post-process/VGUI pass, not a surface shader, with no spec; its material stays unresolved |
| surf_demise's "magenta floor": its ramps are translucent marble over an envmap-only material (no `$basetexture`) drawn grey instead of black (shaders.md 2), and its HDR sky cubemap failed twice (a pak entry the zip crate's LZMA decoder refuses; half-float texels) | 1 | Black albedo for envmap-only generic materials; such pak entries read through lzma-rs; RGBA16161616F decoded |
| Material effects the shaders lacked: `$detail` on models (surf_surreal, kz_ancient_ruins, surf_boreas: 120+ materials) and blend modes past 0 and 1 (2, 5, 6, 8), `$selfillum` (27 maps; also de_nuke's windows), `$basetexturetransform` (22 maps) and TextureScroll on it (21), sky faces' half-height transform, UnlitGeneric brushes lit by the lightmap, `$selfillum` inside `">=DX90"` blocks ignored | 30+ | `map::material_fx`, world.wgsl and prop.wgsl (specs/cs_source/shaders.md; open questions 16-19 list what the spec leaves out); `$emissiveblend*` (surf_demise) still not drawn (question 17); de_nuke refcmp 0.0248 -> 0.0238 (its self-lit windows), de_dust2 unchanged (0.0308) |
| Slow first views: mg_swag_multigames_v1 341 ms (every resting placed weapon ran swept CCD each tick, 19 ms a tick), surf_demise 98 ms (37k static parts revisited by transform propagation whenever anything under the map's root moved) | 2 | CCD only above 0.5 m/s; static parts under a root of their own. performance.md, "Community maps' slow first views", has the traces and before/after numbers |
| Surfing stopped dead on most surf_sedona ramps (high and far from the origin) and at the joints of curved ramps | surf_sedona: 55 of 112 test rides (`heavy::map_surf`) | `movement::Tracer::sweep_brushes`: how far a move goes into a plane is `n . delta` (the end points' f32 rounding made a velocity just clipped along a ramp read as entering it again: the same plane twice zeroed it), and the plane a box stops against is the one it really crosses last (ranking the backed-off fractions picked a curved ramp's seam bevel, facing back along the ramp); prop pieces get edge bevels (`MapBrush::from_planes`). Now no ghost stop on surf_sedona, surf_boreas, surf_apollo, surf_jive |
| surf_sedona: 100 s to load and two minutes more to spawn (2026-10-09) | 1 (others with big LZMA paks or huge triangles less so) | Packed files read by `pak::Pak` (vbsp's zip decoded LZMA at a few MB/s; now lzma-rs, all at once on several threads), LZMA lumps inflated in parallel, and the decal and surface-colour grids keep triangles spanning over 64 cells in a list of their own (a 100 m ramp filled millions of 1 m cells: 70 s and gigabytes). Dev build: load 102 s -> 9 s, spawn 67-137 s -> 4 s. Stage times: `MapData::load_times` |
| surf_sedona's lighting "corrupted", props black: it has HDR lighting only (no lump 8), and lump 7's faces were read through lump 53 (four styles of 0, offsets into nothing) with all-zero LDR ambient samples | 1 | Fullbright below mat_hdr_level 2, as CS:S draws it (`lightmap::select_lighting`, `MapLighting::fullbright`); at level 2 its HDR lighting through the HDR face lump (58), with the leaf ambient index's u16 first sample unwrapped (82293 samples) |

## Map entities (2026-10-09)

From the four specs below (reviewed and approved): player_speedmod,
game_ui, env_fade, game_score, env_hudhint, env_explosion,
func_wall_toggle, func_conveyor (`logic::game`); point_viewcontrol
(`logic::camera`, `core::MapView`); point_template and env_entity_maker
(`logic::templates`, node copies in `map::copies`); func_physbox(_multiplayer)
as physics bodies with the compiler's stored mass, phys_thruster and
phys_keepupright (`logic::physics`, `map::controllers`); point_spotlight,
env_laser, persistent env_beam and env_lightglow drawn (`map::beams`),
env_spark's sparks; entities parented to a placed weapon follow it and
its carrier, OnPlayerPickup (`map::entities` anchors). mg_item_battle_v4b:
walking onto an item knife picks it up, its car rides along under the
player and its speed change applies. Tests: `src/logic/game_tests.rs`,
`template_tests.rs`, `tests/it/map_game_entities.rs`,
`tests/it/heavy/map_community.rs` (`item_children_follow_the_player`).
Shortcuts are in docs/tech-debt.md; the specs' open questions stand.

Sweep (89 maps), entity classes nothing handles: 56 classes / 2945
entities before, 36 / 684 after; no map lost its load. Left of the four
specs: prop_ragdoll (6 maps), env_spritetrail (8), strike-generator
env_beams, parented (dynamic) spotlights and their dynamic light.

## Map logic audit (2026-10-10)

The user saw "a lot of broken features with entities and map logic".
`mapsweep --audit` (`logic::audit`, docs/OBSERVABILITY.md) checks every
output connection of the 89 maps against what the logic handles and runs
a scripted player through every trigger and +use at every door and button,
then a round restart. Raw output stays in `target/mapsweep/{before,after}/`
(`audit.md`, `audit.txt`). Totals (89 maps, 20487 output connections):

| | Before | After |
|---|---|---|
| Connections to inputs the target's class doesn't handle | 2856 (96 kinds) | 1578 (58 kinds) |
| Connections to targets that match nothing | 46 | 46 (map bugs: the names exist nowhere) |
| Entity classes nothing handles | 36 classes / 684 entities | 30 / 592 |
| Children of movers left where they spawned | 440 (28 classes, 20 maps) | 0 |
| Movers that jumped to their goal in one tick | (the before run counted round restarts too) | 0 |
| Movers told to move that didn't | 0 | 0 |
| Complaints the scripted runs logged | 2445 | 2096 |
| Panics | 0 | 0 |

Fixed, by maps affected (tests in `src/logic/community_tests.rs`,
`logic::audit::tests`, `map::copies::tests`, `games::cs_source::sound`):

| Problem | Maps | Fix |
|---|---|---|
| Entities parented to a mover stayed where they spawned: triggers on lifts and trains (teleports, hurts, pushes), teleport destinations, doors, buttons and fans on movers, path_tracks, sprites | 20 (440 entities) | `logic::anchors` links every child of a mover (and of placed weapons, physics brushes) and carries it each tick; mover children keep their own motion in the parent's frame (translation) and carry its velocity for riders |
| Movers parented to anything were baked into the world (no node: func_rotating.Start, func_door.Open did nothing visible) | 5 | Mover classes always get a node (`cs_source::bsp::brush_entities`) |
| SetSpeed 0 on a moving func_movelinear snapped it to its goal (lifts teleported) | 1 (120 connections) | A speed-0 move holds with its goal kept (`movers::Pusher::move_to`) |
| Physics objects never touched triggers (spawnflag 8/64: boats on boosters and finish lines, karts) | 11+ maps have such triggers | `LogicWorld::touch_bodies`: physics props and func_physbox by their collider boxes; trigger_push impulses (`Effect::BodyVelocity`), trigger_teleport (`Effect::BodyTeleport`) |
| AddOutput classname on players ignored (kz_/bhop_ stage marks for filter_activator_class) | 3 | `LogicWorld::player_classes`; AddOutput classname on entities renames them for filters |
| AddOutput maxspeed, force, message, weapon_* on func_rotating, phys_thruster, ambient_generic, game_player_equip did nothing | 3 | Applied (`classes::class_keyvalue`; added sounds are loaded with the map) |
| Color on brushes (selector buttons turning red once taken) | 3 | `map::tint` (rendercolor at load too) |
| func_rot_button, momentary_rot_button, logic_measure_movement, point_teleport, logic_multicompare, env_shake unhandled | 1-3 each | `movers` (rotating buttons) and `logic::community` |
| ForceSpawn of a template holding a func_physbox hung the game and filled memory (mg_creative_multigames_v8_ns's boats; found playing it) | 1+ | `map::copies::copy_node` no longer follows a body's own collider list |

Play-check (live, windowed; `ent_dump` over the remote console,
screenshots in this session's `target/playcheck/`):

| Map | Mechanism | Result |
|---|---|---|
| mg_swag_multigames_v1 | Lifts (func_movelinear with SetSpeed 0 stops, a trigger_teleport parented to each) | Lifts stop between floors and go on; their teleports ride with them |
| mg_lego_multigames_v2 | Minigame selector button (+use) | Pressed and locked; the spawn teleports retargeted (AddOutput target), the case picks |
| mg_creative_multigames_v8_ns | Mode buttons; boat spawners | Buttons yellow -> red when taken; boats spawn (hung before) |
| kz_bhop_izanami | Fall teleports filtered by name/class | A fall in a stage lands on its checkpoint |
| mg_lt_galaxy_v5 | point_teleport moving a spinning 3D-skybox brush, weapon relays | Moves and keeps spinning; the scout relay equips |
| mg_boatrace_scramble | Boats driven by thrusters through boost triggers | Fails: boats sit on the pool floor (no water buoyancy), thrust can't move them; triggers are ready for them |

What still fails, ranked by maps affected (after):

1. Visual entities not drawn: env_spritetrail (8 maps), info_particle_system
   (7; its Start/Stop inputs on 2), env_smokestack (3), prop_ragdoll (6).
2. Players' render inputs (rendermode/renderamt/rendercolor, Alpha, Color on
   `!activator`: invisibility and team colours, 4 maps); brush Alpha.
3. Physics: no water buoyancy (boats: mg_boatrace_scramble,
   mg_creative_multigames_v8_ns), phys_motor (2), phys_constraint and
   ballsocket (2), point_push (1).
4. Players parented to things (SetParent/SetParentAttachment on players:
   mg_crazykart_v1_1's karts), entities SetParent at run time (2 maps).
5. Classes left: func_monitor/point_camera (2), point_tesla (2),
   env_shooter (2), func_tanktrain (2), env_texturetoggle (1, 1056
   connections), env_screenoverlay (1), func_water_analog (5: moving water),
   func_reflective_glass, env_viewpunch.
6. Noise rather than breakage: triggers naming a filter that doesn't exist
   (5 maps, the game lets everyone through, as we do), `sm_say`/`ma_say`
   plugin commands refused, VScript inputs (`RunScriptCode`, CS:GO only).

## Minigame flows (2026-10-10)

The 17 mg_ maps played through headless as players meet them
(`tests/it/heavy/map_flows.rs`: the whole game, rounds on, scripted
characters on both teams pressing selectors with +use, standing in
teleports and arenas, getting loadouts, dying, and the next round
bringing the selection back; `MASHUP_FLOW_TABLE=1 cargo test --features
dev --test it map_flows:: -- --nocapture` prints a row per step,
`MASHUP_FLOW_TRACE=<names>` the inputs and outputs of those entities).
Each map's flow, from its entities:

| Map | Flow |
|---|---|
| mg_lego_multigames_v2 | First into the spawn hall's centre (`telepot_winner`) goes to the selection room; 16 game buttons (one locks all, says the game, retargets the hall's team teleports); dropping back through the centre turns the teleports on; arenas hand out weapons and health on arrival (knife: 35 hp); a 210 s time limit hurt per game; round ends by elimination |
| mg_creative_multigames_v8_ns | Intro camera; first into the hall's laser pole (`tel_sala`) chooses: language (English/Spanish: `mp_restartgame 2`, remembered through a kept func_brush and a physics prop falling into a trigger), then one of 16 modes in 60 s (buttons turn red and lock; the mode relay says it, kills the hall's placed weapons, retargets `st`/`sc`, sets health and loadout through `thp`/`wep`); auto-pick timer; 6 min rounds |
| mg_jacks_multigames_v1 | Random spawn mode at map start (climb, surf or normal course to the lobby); selection room of 11 games (one locks all `button_*`, says it, retargets and turns on 7 team teleporters); leaving the teleporter's volume hands out the game's weapons |
| mg_lt_galaxy_v5 | Countdown, vote at 20 s: 12 pads count who stands on them (math_counter, logic_case setting meter speeds); the first meter at the top starts its game: team teleports, countdown, `[START]`; winners get the winner's loadout, 15 s to the round end |
| mg_swag_multigames_v1 | Spawn room floor opens; first down the middle drop is the chooser (a template spawns 15 game buttons in the game room); a timer picks at random if nobody does; team teleports retarget; arenas equip on arrival; announcements are SourceMod `sm_say` (refused, as on a server without it) |
| mg_randomizer_v5 | 5 s into each round a logic_case picks one of 16 round types: says it, turns on the spawns' weapon triggers |
| mg_n64_goldeneye_v2 | Central room; a player there cancels the bots' auto-start; a weapon button opens the spawn door and its wing's door; 20 s later a beam kills whoever stayed |
| mg_wipeout, mg_wipeout2 | Obstacle courses in tiers: stage-end teleports score (game_score), holding pens fill teleport slots (AddOutput target); wipeout2 has CT spawns only |
| mg_lego_course | T spawns only; spawn door opens at 5 s, AFK hurt at 45 s, random breakable floors, finish line scores |
| mg_escape_prison_beta | T spawns only; trap course (axes, grinders, presses, a touch-activated gas trap) to a bomb site; the last stretch sets a 15 s bomb timer |
| mg_crazykart_v1_1 | T spawns only; random stage, or the last winner (a player named `winner`) picks; countdown; karts (players parented to physics props: another session's) |
| mg_boatrace_scramble | CT spawns only; lobby (glass doors, bowling), jetty starts the lights, start wall drops at 30 s; each boat's starter hands its game_ui (boats need water buoyancy: another session's) |
| mg_item_battle_v4b | P228 at spawn; glass walls break at 8 s ("Fight!"); item knives give powers (OnPlayerPickup hands a game_ui; secondary attack drives it) |
| mg_kommando, mg_starwars_v1 | Vehicle sandboxes: a button hands the presser a vehicle's game_ui (movement keys into logic_compares into thrusters) |
| mg_3k_smash_lego_copter | Jump into the middle: survive 90 s of block spawners, then a random ending (a logic_case presses one of four buttons) |

Steps passing, before -> after this session's fixes (17 maps, 141
steps: 114 -> 141; the one "known" row is below):

| Map | Steps | Before | After | What failed before |
|---|---|---|---|---|
| mg_3k_smash_lego_copter | 5 | 3 | 5 | announcements lower-cased |
| mg_boatrace_scramble | 7 | 5 | 7 | everyone dead: no round end (one team only) |
| mg_crazykart_v1_1 | 6 | 2 | 6 | announcements lower-cased (stage picks, countdown) |
| mg_creative_multigames_v8_ns | 15 | 13 | 15 | mode announcement lower-cased; the hall's placed weapons stayed when the mode killed them |
| mg_escape_prison_beta | 6 | 4 | 6 | the touch-activated gas trap; no round end (one team only) |
| mg_item_battle_v4b | 9 | 7 | 9 | "Fight!" lower-cased; known: an env_fire burns the start glass at load |
| mg_jacks_multigames_v1 | 13 | 12 | 13 | game announcement lower-cased |
| mg_kommando | 5 | 4 | 5 | intro lower-cased |
| mg_lego_course | 7 | 5 | 7 | AFK warning lower-cased; no round end (one team only) |
| mg_lego_multigames_v2 | 13 | 12 | 13 | game announcement lower-cased |
| mg_lt_galaxy_v5 | 10 | 7 | 10 | vote countdown, game and `[START]` lower-cased |
| mg_n64_goldeneye_v2 | 7 | 7 | 7 | |
| mg_randomizer_v5 | 7 | 6 | 7 | round type lower-cased (and every map load picked the same first round) |
| mg_starwars_v1 | 5 | 5 | 5 | |
| mg_swag_multigames_v1 | 11 | 11 | 11 | (its `sm_say` lines are refused either way) |
| mg_wipeout | 8 | 8 | 8 | |
| mg_wipeout2 | 7 | 3 | 7 | intro lower-cased; stage scores lost to the stage teleport; no round end (one team only) |

Fixed, by maps affected (tests: `map_flows`, plus the unit tests named):

| Problem | Maps | Fix |
|---|---|---|
| Every `say` line, game_text and hint shown in lower case: vbsp lower-cases the whole entity lump | 13 of 17 mg_ (every map with announcements) | Output connections and text classes' `message` get the map's case back from the raw lump (`cs_source::bsp::restore_text_case`, `text_keeps_the_maps_case`) |
| Players on one side only (one team's spawns): all dead waited for the round clock (5 min) | 5 (lego_course, escape_prison, crazykart, wipeout2, boatrace) | A draw and the next round (`rules::rounds`, objectives.md Q15, our reading; `rounds::one_side_all_dead_is_a_draw`) |
| The map logic's dice had one fixed seed: every map load made the same random picks | 10 mg_ use PickRandom (randomizer's first round type, crazykart's stage, jacks' spawn mode...) | A clock seed per map load; the harness fixes it (`logic::LogicSeed`) |
| A teleport took the player out of the other triggers sharing its volume (the touch list was re-tested after each touch) | mg_wipeout2's stage scores (any map scoring on a teleport) | Overlapped triggers listed first, then touched (triggers.md "Per-tick order"); a teleport re-links at the destination (`teleport_keeps_the_other_touches`) |
| Placed weapons the logic killed stayed in the world | mg_creative_multigames_v8_ns (every mode clears the hall's) | `map::entities::RemovedWeapons` from the bridge, `weapon::equip::remove_killed_weapons` (loose or carried) |
| `phys_pushscale` missing (football: the map sets 900, hits barely moved the ball); maps' value clamped at 100 | 4 (creative, lego_multigames, swag, randomizer) | `weapon::PushScale` multiplies shot, swing and blast pushes (physics_props.md 5; `ragdoll::push_scale_multiplies_shot_pushes`); bound 1000 |
| func_button "Touch Activates" (256) did nothing | 1 (escape_prison's gas trap) | Pressed by a player against it (doors_buttons.md; `touch_activated_buttons`) |

Play-check (live, windowed, 5 bots, rounds on; screenshots in this
session's `target/playcheck/`): mg_lego_multigames_v2 (chooser ->
selection room, Knife: arena, knife, 35 hp; round end, buttons and
teleports reset; Football next round), mg_creative_multigames_v8_ns
(intro camera, chooser, English: `mp_restartgame` and the language kept,
Pirate War: buttons red, ships, 400 hp), mg_jacks_multigames_v1 (surf
spawn mode, Mp5Deagle: buttons locked, arena, deagle), mg_lt_galaxy_v5
(vote pad, meter climbs, bunnyhop course), mg_swag_multigames_v1
(drop to the game room, timeout pick: Airdrop arena, knife),
mg_n64_goldeneye_v2 (knife wing button: spawn and wing doors open; the
beam kills the bots who stayed; next round). All flowed.

What still blocks, by maps affected:

1. **Players teleported onto one destination stay stuck in each other**
   (every selector map sends a team to one info_teleport_destination;
   seen live on all six). Players are solid boxes to each other and the
   stuck nudges (movement.md) are a few units. CS:S likely does the same
   (servers run "noblock" plugins for it); unmeasured. Not round 2's: a
   movement spec question (two players teleported onto one spot: can
   they walk apart?) and then, per "parity first", an opt-in
   no-block toggle.
2. **Bots don't play minigames**: they stand or wander (no nav mesh on
   most mg_ maps); rounds end only by map hurts or the clock.
3. **SourceMod/Mani announcements** (`sm_say`, `ma_csay`): refused, as a
   server without the plugin would (swag, boatrace winners, item_battle).
4. mg_item_battle_v4b's start glass burns at load: an env_fire (flags 4,
   8) touches the glass brush's arena-wide box (fire.md 2.3 step 7), so
   the walls are gone before "Fight!"; a fire spec question (line of
   sight or bounds for brush entities).
5. Done in round 2 (below): crazykart's karts (players parented),
   boatrace's and creative's boats (buoyancy), item powers' visuals
   (spritetrails, particles). `mp_flashlight`/`sv_alltalk` unknown
   settings (logged) remain.

## Map logic audit, round 2 (2026-10-10)

Working down round 1's ranked list ("What still fails", above): visual
entities, render looks, physics in water and between bodies, players
parented to things, the remaining classes, and the scripted runs' noise.
`mapsweep --audit` again over the 89 maps (raw output in this session's
`target/mapsweep/round2/`; round 1's "after" is the "before" here):

| | Before (round 1 after) | After |
|---|---|---|
| Connections to inputs the target's class doesn't handle | 1578 (58 kinds) | 31 (9 kinds: map mistakes and HL2/later-engine inputs, below) |
| Connections to targets that match nothing | 46 | 46 (the map's own) |
| Entity classes nothing handles | 30 classes / 592 entities | 13 classes / 43 entities |
| Complaints the scripted runs logged | 2096 | 103 (10 kinds) |
| Notes (known, intended gaps, once per map: `LogicWorld::note`) | (counted as complaints) | 115 (7 kinds) |
| Usable brushes +use didn't find | (every mover tried) | 7 (only solid ones tried now) |
| Movers that didn't move / jumped / children left behind / panics | 0 / 0 / 0 / 0 | 0 / 0 / 0 / 0 |

Fixed, by maps affected (tests in `src/logic/community_tests.rs`,
`map::emitters`, `map::psys`, `games::cs_source::pcf`, `map::tint`,
`map::buoyancy`, `map::controllers`, `tests/it/logic_physics.rs`,
`tests/it/heavy/map_community.rs::visual_entities_load`):

| Problem | Maps | Fix |
|---|---|---|
| env_spritetrail not drawn | 8 (59 trails) | `map::emitters::TrailEmitter` (visual_entities.md 5) through the particle pool; drawn parts follow their entity when it follows a parent (`FollowsEntity`, sprites too) |
| info_particle_system not drawn | 7 (307 systems) | `.pcf` (binary DMX 2) from the install's manifest and the map's pak (`games::cs_source::pcf`), run by `map::psys` (29 operator kinds, children, Start/Stop instances); surf_hellenic's 160 fires, crazykart's kart effects, surreal's, boreas', threnody's (surf_halloween_tf2's TF2 systems aren't in CS:S's files: not found, logged) |
| prop_ragdoll not placed | 6 (10) | Drawn in Hammer's pose ("angleOverride"), else lying on the floor below it; not simulated |
| env_smokestack not drawn | 3 (20) | `map::emitters::SmokeStackEmitter` (particles_and_smoke.md 3) |
| Players' render looks (invisibility, colours: AddOutput rendermode/renderamt/rendercolor, Alpha, Color on `!activator`) | 4 (23+26+23 connections, 362+321 run-time lines) | `LogicWorld::player_looks` -> `map::tint::RenderLook` on the character: body and held model fade, add or tint |
| Brush alpha and render modes | 7 brush entities' maps (surf_demise's 51 faint panels, kz_bhop_sakura, mg_creative) and Alpha/AddOutput renderamt at run time | Baked brush entities blended from load (`MapMesh::render`); nodes by `BrushTint` (look and texture frame) |
| Boats sank (no buoyancy); thrust couldn't move them | 2 (mg_boatrace_scramble, mg_creative's boats) + every physics prop in water | `map::buoyancy`: lift by the colliders' volume and wet fraction, water drag; boatrace's catamaran does ~270 units/s on its thruster |
| phys_motor, constraints (ballsocket, constraint; hinge, slide, length too) | 4 (wipeout2, surreal, bk_warehouse, churches) | `logic::physics` motors and joints -> `map::controllers` (motor spin, avian joints, Break/TurnOff) |
| point_push | 1 (28) | Physics bodies and players pushed while enabled |
| Players parented to things (SetParent/SetParentAttachment/ClearParent on players and entities at run time) | 3 (crazykart's 64+64 connections, creative's trails and spinners, crazykart's particles) | `anchors`: run-time parents, players ride their parent (`MapControls::parented`: no own movement, no physics shadow or push-away, so the kart isn't broken by its rider), model attachments from the loader (`$attachment` keys); moving physics props move their children whatever their classname |
| func_monitor/point_camera | 2 (lt_galaxy's 24 screens, kommando) | `map::monitor`: a render target of the active camera's view |
| point_tesla | 2 | Arcs to surfaces (or into the air) through the particle pool |
| env_shooter | 2 | Its model as gibs |
| func_tanktrain | 2 | A func_tracktrain |
| env_texturetoggle | 1 (1056 connections) | Movers' texture frames |
| env_screenoverlay | 1 | A picture over every screen (`HudShow::Overlay`; network clients get it with the other HUD events) |
| func_water_analog | 5 | Its water is swimmable where it spawns; its inputs accepted (it doesn't move) |
| Noise: VScript and later-engine inputs, server plugin commands, missing filters, names nothing has, round restarts, client commands off the allowlist | most maps | Noted once per map (`LogicWorld::note`), apart from complaints; real ones fixed: a damage filter naming nothing clears it, env_entity_maker's AddOutput EntityTemplate, env_soundscape_triggerable, func_reflective_glass, props' damage-force inputs |

Play-check (live, windowed; console over the remote port; screenshots in
this session's `target/scratch/shots/`):

| Map | Mechanism | Result |
|---|---|---|
| mg_boatrace_scramble | Boats in the boathouse; a catamaran's accelerate thruster | Boats float at the waterline; the catamaran leaves its berth at ~270 units/s and runs down the channel (`boat_before.png`, `boat_after.png`) |
| mg_crazykart_v1_1 | Step onto a kart (its starter trigger): SetParent to the seat, game_ui, then drive | The player is snapped to the kart's seat and rides it as it drives and turns; the kart no longer breaks under its rider (`kart_garage.png`) |
| gg_future | Lasers on a func_rotating with env_spritetrails | Four coloured trail rings (`gg_future_trails.png`) |
| surf_hellenic | env_fire_large info_particle_systems (fire_01.pcf from the install) | Flames, smoke and embers on the braziers (`hellenic_fire3.png`, `hellenic_fire4.png`) |
| gg_nukkon_hdr | env_smokestack | Cooling towers smoke, drifting with the wind (`nukkon_smokestack.png`) |
| mg_lt_galaxy_v5 | func_monitors and point_camera | The 24 sphere screens show the camera's view (`galaxy_monitors.png`) |
| surf_halloween_tf2 | point_tesla | Purple arcs (`halloween_tesla.png`) |
| gg_deagle7k | prop_ragdoll without a pose | Lies on the floor (`deagle_ragdoll2.png`) |

What still fails, ranked by maps affected (after; `target/mapsweep/round2/audit.md`):

1. **Map ragdolls aren't simulated** (6 maps, 10): drawn posed or lying,
   not falling, not solid; the character ragdoll code (`map::ragdoll`)
   is tied to characters.
2. **Classes nothing handles** (13 classes, 43 entities): env_detail_controller
   (3 maps), env_embers (2), color_correction (2), env_wind (2),
   propper_model (1, 14: a compile-time tool entity), info_ladder_dismount,
   env_muzzleflash, env_viewpunch, func_fish_pool, info_constraint_anchor,
   ai_changetarget and logic_script (HL2/CS:GO, absent in CS:S too),
   surf_happyhands' odd ambient_generic.
3. **Particle effects are interpretations** of the `.pcf` operators
   (tech-debt row): their look isn't compared with CS:S; surf_halloween_tf2's
   37 systems are TF2's (not in CS:S's files: not drawn there either).
4. **Moving water** (func_water_analog motion: mg_jacks_multigames_v1's
   rising flood; water parented to movers on 2 maps) stays where it spawns.
5. **Run-time complaints left** (10 kinds, 103 lines): player
   SetFogController (surf_halloween_tf2, 79), ambient_generic presets
   (2 maps), sv_cheats/rcon refused (3, on purpose), and map mistakes
   (inputs CS:S entities don't have: trigger_once PlaySound,
   env_soundscape PlaySound, func_breakable Open; AddOutput without a
   value; a track train without a path; a template without members).
6. **Monitors**: one camera at a time, no 3D skybox or fog in it (both
   since rounds 3 and 4), culled for the player's view; teslas and other effects inside a 3D skybox
   draw at their skybox place.

## Map logic audit, round 3 (2026-10-10)

Round 2's ranked list ("What still fails", above), top down (course
flows on surf_/bhop_/kz_/gg_ maps were another session's). `mapsweep
--audit` again over the 89 maps (raw output in this session's
`target/mapsweep/round3/`; round 2's "after" is the "before" here):

| | Before (round 2 after) | After |
|---|---|---|
| Connections to inputs the target's class doesn't handle | 31 (9 kinds) | 22 (5 kinds: map mistakes, below) |
| Connections to targets that match nothing | 46 | 46 (the map's own) |
| Entity classes nothing handles | 13 classes / 43 entities | 1 class / 1 entity (surf_happyhands' ambient_generic without a sound: removed, as the game does) |
| Complaints the scripted runs logged | 103 (10 kinds) | 18 (7 kinds: map mistakes, sv_cheats/rcon refused) |
| Notes (known, intended gaps, once per map) | 115 (7 kinds) | 129 (16 kinds) |
| Usable brushes +use didn't find | 7 | 0 (5 visits where the map moves the player away, listed apart) |
| Movers that didn't move / jumped / children left behind / panics | 0 / 0 / 0 / 0 | 0 / 0 / 0 / 0 |

Fixed, by maps affected (tests: `tests/it/heavy/map_community.rs`
`map_ragdolls_fall_and_take_shots`, `moving_water_follows_its_entity`,
`visual_entities_load`; `tests/it/net_interp.rs`
`map_ragdolls_are_drawn_as_the_server_simulates_them`;
`tests/it/logic_physics.rs` (anchor, force limit); `src/logic/tests.rs`
`use_reaches_a_usable_parent`, `water_analog_moves`;
`src/logic/community_tests.rs`
`view_punch_muzzle_flash_color_correction_and_noted_classes`;
`map::color_correction`, `map::fish` unit tests):

| Problem | Maps | Fix |
|---|---|---|
| Map ragdolls weren't simulated (drawn posed or lying) | 6 (10 ragdolls, all with a ragdoll `.phy`, all debris; none has a Hammer pose) | `map::placed_ragdoll`: the character ragdoll's bodies, joints, settling and repair from the first sequence at the entity's angles (or Hammer's pose); debris on the ragdoll layer: players pass, shots push the part they hit; 16384/65536, EnableMotion, DisableMotion, Wake; killed and remade at a round restart with the logic; parts sent to network clients (`net::ragdolls`, `NetRagdoll`) |
| Usable brushes +use didn't find | 7 (3 maps) | None were +use's: crazykart's 5 stage buttons sit in the winner's room, whose triggers (trigger_hurt "hurt_spawn"/"hurt_all" around it) take the scripted player away before it presses (the flows test presses them as the winner); n64's spawn door is one func_door of four panels, and the audit aimed at the empty middle between them; the lego func_rotating is found now that the audit aims at one of its pieces. The audit now aims at a piece and lists "the map moved the player away" apart. Also fixed per doors_buttons.md: a hit brush that isn't usable tries its parent chain (`find_use`) |
| Moving water stayed where it spawned | 3 (mg_jacks_multigames_v1's flood: up 635 units at 11/s; water parented to a spinning rotator on mg_3k_smash_lego_copter and to a door on mg_creative_multigames_v8_ns, whose surfaces were drawn at the world origin) | func_water_analog is a func_movelinear; its swim volume (`MapData::water_movers`, `water::move_water`) and surface (drawn under its node) follow the node, as water parented to a mover does: swimming, buoyancy, splashes and the under-water view all read the moved volume |
| color_correction not applied | 2 (mg_lt_galaxy_v5, surf_demise) | `map::color_correction` (the pak's `.raw` 32³ tables, red fastest; weight by falloff distance, fades) blended into one 3D table; `client::color_correction` applies it after the tone map (`mat_colorcorrection`) |
| env_embers not drawn | 2 (jacks' 6, 3 on; surf_stickybutt_alpha's shaft) | `emitters::EmbersEmitter` (TurnOn/TurnOff/Toggle) |
| env_wind, env_detail_controller, info_ladder_dismount, propper_model, logic_script, ai_changetarget | 2, 3, 1, 1 (14), 1, 1 | Noted once per map (`classes::noted_class`), with their inputs: nothing to do in CS:S or nothing we draw (detail props aren't drawn; only kz_ancient_ruins has any) |
| env_muzzleflash, env_viewpunch, func_fish_pool, info_constraint_anchor | 1 each (kommando, surreal, kommando, jacks) | Flash sprites on Fire; a view kick in its radius (`map::ViewKick`; its roll isn't kept); 20 goldfish swim about the pool (`map::fish`); constraints resolve an anchor's name to its parent's body (jacks' has no parent: inert, as in the game) |
| Constraints never broke by force | 4 (the constraint maps) | Past forcelimit/torquelimit (avian's joint forces against the limit as a weight in the map's gravity) the joint goes, OnBreak fires (`controllers::JointBroke`) |
| Monitors had no sky behind; skybox effects drawn tiny at their skybox place | monitors 2, skybox teslas 1 (galaxy's tower beams) | A monitor sky camera (2D sky and 3D skybox from the camera's place); particles outside the playable area go in a mesh on the sky camera's layer |
| Player SetFogController (79 complaints), ambient_generic presets | 1, 2 | Noted: halloween's names no fog controller (nothing happens in the game either); the preset table isn't in sounds.md (Q10), the keys play |

Play-check (live, windowed; screenshots in this session's
`target/scratch/shots/`):

| Map | Mechanism | Result |
|---|---|---|
| gg_deagle7k | prop_ragdoll (corpse, no pose) | Falls from its placed pose and lies spread on the floor (`deagle_ragdoll.png`) |
| mg_jacks_multigames_v1 | `ent_fire water Open` | The flood rises up the lobby's steps (`jacks_flood_before.png`, `jacks_flood_after.png`) |
| mg_lt_galaxy_v5 | color_correction | The station's reds go magenta, greys blue, the sun red and yellow; `mat_colorcorrection 0` shows the plain frame (`galaxy_cc_on.png`, `galaxy_cc_off.png`) |
| surf_stickybutt_alpha | env_embers (pitch 90) | Cyan embers fall down the shaft (`stickybutt_embers.png`) |
| mg_kommando | func_fish_pool | Goldfish swim in the pool (`kommando_fish.png`) |
| mg_lt_galaxy_v5 | Monitors' sky, skybox tower teslas | Not seen: its point_camera is in a closed room (no sky to show), and the tower beams weren't on from where we looked; untested beyond the code |

What still fails, ranked by maps affected (after; `target/mapsweep/round3/audit.md`):

1. **Particle effects are interpretations** of the `.pcf` operators
   (tech-debt row); embers' and muzzle flashes' looks are ours too.
2. ~~**Detail props aren't drawn**~~ (drawn since round 4, below; the
   env_detail_controller and env_wind keys feed them).
3. **Map mistakes the audit shows** (inputs CS:S entities don't have:
   jacks' func_rotating AddOutput EntityTemplate, trigger_once and
   env_soundscape PlaySound, func_breakable Open, math_counter Unlock;
   AddOutput without a value; a track train without a path; a template
   without members), sv_cheats/rcon refused on purpose.
4. **Monitors** use the map's fog, not the point_camera's own keys
   (their own since round 4, below); one camera at a time.
5. env_viewpunch's roll (surf_surreal's crash punch is all roll).

## Course flows (2026-10-10)

The 30 surf_, bhop_ and kz_ maps and the 42 gg_ maps played through
headless (`tests/it/heavy/map_courses.rs`, one test per map; how it
works and its switches: docs/OBSERVABILITY.md after "Minigame maps'
flows"). Nothing is per map: every check is derived from the map's
entities, so each row counts items (teleports, checkpoints, blocks,
boosters...), and items the harness can't put a player into are "known"
with the reason (890 of 4199, below). `heavy::map_surf` also rides every
open ramp of 14 more surf maps. Each map's course, from its entities:

| Map | Flow |
|---|---|
| bhop_addict_v2_3xl | T spawns only; 157 teleports to 46 stage starts; 109 "multihop" blocks (a trigger on each names the lander `activator` 0.09 s later and `default` at 0.1 s, over a teleport filtered by that name: stand still and you go back); 35 trigger_push boosters (600 sideways, 2500 up); 5 AWPs placed; a button spawns a template |
| bhop_backport_css | CT only; 63 classic bhop blocks: func_doors that open when touched (flag 1024), drop at speed 25 and come back after 0.1 s, over 18 fail teleports to 8 starts |
| bhop_flatzone | 23 teleports to 13 starts; plain blocks over teleport floors |
| bhop_myztek | 115 teleports to 50 starts; stage triggers name the player `filter_tele_N`, the stage's fail teleports let only that name through (24 names); 46 multihop blocks; 15 booster pads (OnEndTouch basevelocity 0 0 300-420); a 1000 up push |
| kz_11342 | CT only; every spawn stands in `tp_start` (all players land on one spot); 128 triggers set gravity 40 on entry and 1 on exit (no-jump zones) |
| kz_ancient_ruins | 11 teleports (3 multihop), stage starts exactly on floors; a fade trigger whose env_fade moves the player (AddOutput origin); a start button |
| kz_bhop_izanami | 479 teleports to 122 starts; 28 stage names, 101 multihop blocks; gravity 40/1 zones and negative-gravity lifts (-0.85); players renamed by class (`A1`, `A2`) for class filters; 3 pushes up; "Good job" game_text at the end |
| kz_bhop_sakura | CT only; 95 teleports to 28 starts; 20 stage names; 13 multihop and anti-prespeed (`pre`, named after 0.8 s) triggers; 9 pushes up (1800-11000) |
| kz_bhop_skodna | 213 teleports to 69 starts; 129 multihop blocks; 19 booster pads (OnEndTouch 0 0 400-600); gravity 40/1 zones and a 0.3 low-gravity pit walled by gravity 1 triggers; class marks (`player_can_use_booster`...) for class filters |
| kz_hikari_od_nh_v2 | 89 teleports to 11 starts (CS:GO-era keys: flags 4097, UseLandmarkAngles); a climb start button |
| kz_rockb1ock | CT only; 4 touch doors (wait 2), 5 teleports, a push, a hurt, a timer button |
| surf_apollo | 139 teleports to 9 starts; stage triggers PressIn/PressOut 34 buttons (stage lights); basevelocity boosters and a gravity -1 zone |
| surf_boreas | 21 teleports to one start; speedmod triggers; momentary_rot_buttons; a func_tanktrain (round 2's) |
| surf_botanica | 232 teleports to 13 starts; gravity -1/1 zones; OnEndTouch boosters (0 0 2000, 0 -200 300) |
| surf_demise, surf_threnody, surf_jive, surf_hellenic | teleports to 2-8 starts (hellenic: landmarks); gravity resets; jive: stage names; hellenic and jive: Momentum-mod timer triggers (not CS:S classes) |
| surf_halloween_tf2 | 322 teleports, 318 filtered by team filters; landmark; particles, point_tesla (round 2's) |
| surf_happyhands | 56 teleports; hud hints per stage; a push up; a wall toggle |
| surf_holiday, surf_kismet, surf_sacrifice, surf_slob | boosters: pushes (2400-3000) and pads (OnStartTouch launch pads 2000 sideways on kismet; OnEndTouch lifts); slob: Momentum timer triggers |
| surf_inferno | a `bonus` course: its start names the player, its boosters (basevelocity up to -4100 0 900) and gravity -1 zone let only `bonus` through |
| surf_nebula | 52 teleports; stage triggers open and close 8 func_movelinear start gates; env_fades; ramps partly Propper-made props |
| surf_nsz_fix | 45 teleports; speedmod triggers; trigger_gravity; buttons that lock (sounds) |
| surf_sedona | 82 teleports to 21 starts (5 with landmarks); 20 pushes; buttons counting into math_counters that open a door; a tracktrain; CS:GO VScript and collectibles inputs (refused) |
| surf_stickybutt_alpha | a bonus chain: each stage's end teleport needs the previous stage's name (`cp1`...`cp6`) and gives the next; a spawn trigger resets to `default`; 4 pushes |
| surf_surreal | 35 teleports to 22 starts; buttons Display game_texts (the paintings); rotating doors, logic_timers; phys_motor (round 2's) |
| gg_ (42 maps) | Arenas: spawns both sides, the default kit unless a game_player_equip gives one (gg_usp_deagle knife+usp, gg_future knife); placed weapons (gg_dinoiceworld 34, gg_iceworld_l33t 37); teleports (gg_mario_vs_wario 4, gg_tbr_water_basin 2, gg_usp_deagle's secret room); gates that rise when touched or by a trigger (gg_towerwars_v2), movelinear platforms (gg_fusion_trx, gg_ilu's buttons); breakables. The gungame itself (a weapon ladder per kill) is the GunGame server plugin's, not the map's |

Steps passing, before -> after this session's fixes (72 maps, 375
steps: 360 -> 375; items 3237 -> 3309 of 4199, 890 known). The gg_ maps
passed every step before and after (spawns, kit, placed weapons,
teleports, triggers, doors, buttons, rounds); the course maps:

| Map | Steps | Before | After | Items before -> after | What failed before |
|---|---|---|---|---|---|
| bhop_addict_v2_3xl | 11 | 10 | 11 | 221 -> 222 of 266 | a stage start hung above its floor |
| bhop_backport_css | 7 | 7 | 7 | 45 of 50 | |
| bhop_flatzone | 6 | 6 | 6 | 34 of 37 | |
| bhop_myztek | 10 | 9 | 10 | 193 -> 208 of 232 | booster pads 0.75 % weak (15) |
| kz_11342 | 6 | 6 | 6 | 95 of 136 | |
| kz_ancient_ruins | 9 | 8 | 9 | 17 -> 19 of 24 | two stage starts hung above their floors |
| kz_bhop_izanami | 11 | 10 | 11 | 520 -> 522 of 751 | a stage start hung |
| kz_bhop_sakura | 11 | 11 | 11 | 142 of 166 | |
| kz_bhop_skodna | 10 | 8 | 10 | 369 -> 391 of 420 | three stage starts hung; booster pads weak (19) |
| kz_hikari_od_nh_v2 | 6 | 6 | 6 | 97 of 103 | |
| kz_rockb1ock | 9 | 9 | 9 | 19 of 20 | |
| surf_apollo | 9 | 7 | 9 | 185 -> 197 of 213 | launch pads touched standing gave no speed at all; a teleport lost the velocity (hung at its source) |
| surf_boreas | 7 | 7 | 7 | 33 of 33 | |
| surf_botanica | 7 | 6 | 7 | 176 -> 177 of 261 | a lift pad weak |
| surf_demise | 6 | 6 | 6 | 20 of 23 | |
| surf_halloween_tf2 | 6 | 6 | 6 | 162 of 333 | |
| surf_happyhands | 7 | 7 | 7 | 51 of 73 | |
| surf_hellenic | 6 | 6 | 6 | 21 of 24 | |
| surf_holiday | 7 | 6 | 7 | 16 -> 19 of 20 | lift pads weak |
| surf_inferno | 7 | 6 | 7 | 11 -> 12 of 16 | the bonus launch pad gave nothing standing |
| surf_jive | 5 | 5 | 5 | 77 of 89 | |
| surf_kismet | 6 | 5 | 6 | 24 -> 28 of 36 | the start's launch pads (2000) gave nothing standing |
| surf_nebula | 6 | 6 | 6 | 58 of 60 | |
| surf_nsz_fix | 10 | 10 | 10 | 59 of 64 | |
| surf_sacrifice | 6 | 5 | 6 | 59 -> 61 of 77 | pads gave nothing standing |
| surf_sedona | 10 | 9 | 10 | 118 -> 119 of 142 | a stage start hung |
| surf_slob | 7 | 6 | 7 | 48 -> 54 of 76 | pads weak or nothing |
| surf_stickybutt_alpha | 8 | 8 | 8 | 28 of 76 | |
| surf_surreal | 9 | 9 | 9 | 81 of 84 | |
| surf_threnody | 5 | 5 | 5 | 108 of 136 | |

The 890 known items: 502 triggers the harness finds no room for a
standing player in (thin slabs inside blocks and floors, triggers in
walls; not checked), 166 filtered teleports no name or class the map
gives passes (mostly surf_halloween_tf2's team filters), 153 teleports
overlapping another one (touch order, triggers.md open question 3), 42
destinations inside another teleport (chained, as the spec says), 8 a
map's missing destination, the rest single cases (gravity zones
overlapping, a booster under a teleport, a multihop teleport below its
block's top). Surf rides (`heavy::map_surf`): no ghost stop on 18 of 19
cached surf maps (14 newly covered); surf_nebula is left out (below;
all 19 since round 4).

Fixed, by maps affected (tests: `map_courses`, plus those named):

| Problem | Maps | Fix |
|---|---|---|
| A player put exactly on a floor (teleport destinations at floor height, how kz and bhop maps place them) hung there: our sweeps found the box start solid (no ground, no fall, no jump), our stuck test didn't, so nothing moved it, for good | 6 seen (bhop_addict_v2_3xl, kz_ancient_ruins, kz_bhop_izanami, kz_bhop_skodna, surf_apollo, surf_sedona; 9 stage starts), any map with such a destination | Touching a plane within float noise (`SOLID_SKIN`) is touching from outside (`Tracer::sweep_brushes`; movement.md open question 18, our reading; `source_movement::feet_exactly_on_a_floor_stand`) |
| `AddOutput basevelocity` on players (booster pads) added to the velocity at once and took the player off the ground: launch pads touched standing gave no speed at all, the rest were 0.75 % short | 9 (13 maps use them) | It sets the base velocity, which the next move takes in x (1 + dt/2) (triggers.md trigger_push step 1, open question 9; `map_logic::basevelocity_booster_launches_by_the_base_velocity_rule`) |
| Classic bhop blocks (touch-open func_doors) didn't drop under a player who landed on them: a landing rests up to 2 units above what it stands on, past the 1-unit touch reach (found live on bhop_backport_css) | 2 (bhop_backport_css 63 blocks, kz_rockb1ock 4) | Standing on the door (its ground) touches it (`movers::touch_movers`; doors_buttons.md open question 9, our reading; `map_logic::landing_on_a_touch_door_opens_it`) |

Play-check (live, windowed, dev build; remote console `setpos`,
`+forward`, `+jump`, `ent_dump`; screenshots in this session's
`target/agent/shots/`):

| Map | Mechanism | Result |
|---|---|---|
| bhop_backport_css | Landing on a bhop block door | Before the door fix it stayed `Closed` under the player; after: it drops (25 u/s), the player falls into the teleport, is back at the start in 0.3 s, the door comes back up |
| bhop_myztek | Multihop block; booster shaft | Landed and stood still 1.8 units above the block: not sent back (see below); one step on it: back at Tele04, view snapped to 270. The OnEndTouch pad launched a jump to about 120 units (57 without) |
| kz_ancient_ruins | Stage 6 start exactly on its floor | Stands, jumps (53 up) and lands (hung in the air before) |
| gg_towerwars_v2 | Gate trigger | Walking into it raises the gate to 190 (Open) |
| surf_kismet | Start launch pad (OnStartTouch basevelocity 2000 0 0) | Landing on it launches the player 450 units along x in a quarter second, into the course's start teleport (the headless check got no speed from it before) |

What still blocks, by maps affected:

1. **The gungame mode** (42 gg_ maps): the weapon ladder (a new weapon
   per kill, knife last) is the GunGame server plugin's; mashup has no
   such mode, so these play as plain arenas with the default kit. A game
   mode for Lucker Party, not map logic.
2. **Touch order of overlapping triggers** (triggers.md open question
   3): 153 teleports overlap another (kz_bhop_skodna's stage 7: a
   filtered and an unfiltered one in one volume, surf_stickybutt_alpha's
   bonus end, ...), 3 gravity zones (skodna's low-gravity pit walls). We
   touch in entity order, each one; which wins in CS:S is unmeasured
   (round 4: no public source settles it; the probe is under "Probes
   wanted").
3. **Landings rest up to 2 units above floors** (movement.md, ground
   detection, no snap): a player who lands on a bhop block and stands
   still can stay above a 1-unit trigger slab on its top (seen on
   bhop_myztek: stood 1.8 above, not sent back until a step). Bhop
   players keep moving, so it rarely shows; two bhop_addict_v2_3xl
   blocks have their teleport entirely under the block's top, which a
   standing player never overlaps (face contact, triggers.md open
   question 1). Whether CS:S snaps a landing down is worth a probe
   (round 4: the spec says it doesn't, and we match it; the probe is
   under "Probes wanted").
4. **Players teleported onto one spot stick in each other** (the
   minigame flows' item 1): kz_11342 puts all 50 spawns in `tp_start`,
   bhop_myztek's spawn room sends everyone to one start.
5. ~~**surf_nebula**: 13 of 84 ramp rides stop dead~~ (round 4: its
   ramp props have no collision model; fixed below).
6. **Timers**: surf/kz timers are server plugins (and Momentum mod's
   `trigger_momentum_timer_*` on 4 surf maps, not CS:S classes): start
   and end zones run their map logic, no timer shows.
7. Visual and physics entities on these maps (func_tanktrain on
   surf_boreas, point_tesla and particles on surf_halloween_tf2 and
   surf_hellenic, phys_motor on surf_surreal, func_water_analog on
   gg_simpsons_dusty_2, spritetrails on bhop_addict, surf_stickybutt,
   gg_future, gg_fy_tactic_fight) are round 2's (above); this session's
   checks don't cover them.
8. surf_sedona's CS:GO inputs (RunScriptCode, AddCollectible) are
   refused, as CS:S would; its collectible counter doesn't count.

## Round 4 (2026-10-10)

Working down the course flows' list and the round-2 leftovers.

| Problem | Maps | Fix (tests) |
|---|---|---|
| 13 of surf_nebula's 84 ramp rides stopped dead. `MASHUP_SLIDE_DEBUG` (new: every slide move's sweeps) showed each stop: a full move whose end tested solid against a prop's collider, or a sweep starting inside one. The props are its Propper ramp models (91 of its 230 static props), which ship no collision model; they sit a hair off player-clip ramps that are the real surfaces, and we collided with their triangle meshes | surf_nebula (91 props); kz_hikari_od_nh_v2 (206 props), gg_fy_tactic_fight (55), surf_halloween_tf2 (44) have such props too | A static prop set to collide by its physics model blocks nothing without one (our reading of the public prop_static docs; tech-debt, probe below). `heavy::map_surf` rides all 19 cached surf maps now (5 to 88 open ramps each, 0 ghost stops), surf_nebula included |
| Detail props not drawn (the BSP's detail lump: grass, weeds, bushes) | 7 cached maps and 3 stock with sprites (cs_militia, cs_compound, de_port), de_inferno, de_nuke, de_train and gg_churches with detail models; kz_ancient_ruins' lump is LZMA-compressed | `games::cs_source::detail` reads the `dprp` lump (compressed or not); sprites and their cross and tri shapes become `MapDetailProps`, drawn by `map::detail` in one mesh per 1024-unit cell (a few draw calls; the vertex shader turns facing sprites, sways tops in env_wind, fades by distance, dithered); detail models become non-solid static props; env_detail_controller's fade, else cl_detaildist/cl_detailfade's defaults (`map_visuals::detail_props_load`, `map_community::detail_sprites_on_community_maps`, unit tests). Screenshots (this session's `target/agent/keep/`): `militia_grass.png`, `ruins_grass.png` |
| Monitors drew the map's fog | point_camera on mg_kommando, mg_lt_galaxy_v5 (fog off on both) | The camera's own fogEnable/fogColor/fogStart/fogEnd/fogMaxDensity (`LogicWorld::monitor_fog` -> `MonitorFog` -> the screen camera's `DistanceFog`, read by `fog.wgsl` as the view's fog; `community_tests::monitor_camera_fog_is_the_cameras_own`; test_hardware's monitors checked live, `monitors.png`) |
| Bots stood on spawns and teleport destinations on maps without a navigation mesh | the 83 cached maps without one (6 pack a mesh) | `bot::aside`: they step off spawns and teleport destinations and out of teleport triggers to a clear spot within 320 units and wait; the console says once that the map has no mesh (`bot_nav::bots_stand_aside_without_a_nav_mesh` on bhop_flatzone). Full minigame AI stays out of scope |

`mapsweep --audit` after (with round 3 merged): as round 3 left it, 22
connections to inputs their class lacks (map mistakes), 46 to missing
targets, 1 unhandled class (1 entity), 18 run-time complaints, 129 notes,
0 movers failing; env_detail_controller and env_wind stay noted
classes, their notes now saying the loader reads them.

Course flows after (72 maps): every step passes; items 3309 -> 3330
(of 4199 -> 4201; surf_botanica 177 -> 193 of 261, surf_nsz_fix 59 of
64 -> 63 of 66, surf_sacrifice 61 -> 62: triggers the solid prop meshes
left no room in). Known: 475 no room for a player (502 before), 166
filtered, 153 overlapping teleports, 42 chained. Minigame flows: 17 of
17 maps pass.

Looked at, not changed:

- **Landing height** (movement.md, ground detection and staying on the
  ground): a player is on ground when a 2-unit drop test finds a floor,
  and the test doesn't move the origin; only a ground move (speed 1 or
  more) snaps it down. So a player who drops straight onto a floor and
  stands still rests where the test caught it, up to 2 units above
  (the spec lists it as a quirk to keep); one landing with any
  horizontal speed is snapped on its first ground tick. We do exactly
  this (`Mover::categorize`, `stay_on_ground`), so nothing changed:
  bhop_myztek's stand-still above a multihop slab and bhop_addict's
  teleports under a block's top are what the spec predicts, unless CS:S
  differs (probe A).
- **Touch order of overlapping triggers** (triggers.md open question
  3): the spec leaves the engine's enumeration open and no public page
  settles it; we touch in entity order (probe B).

Probes wanted (the reference server; not run here):

- A, landing height (movecmp): a flat brush floor at z = 0. `setpos` the
  feet at z = 1.0, 1.5, 1.9 and 2.5 with no velocity and no input; read
  `getpos` every tick for half a second. Spec: 1.0-1.9 stay put (on
  ground at once), 2.5 falls 0.098, 0.391 (z 2.11), then is caught at
  z 1.62 and stays. Then drop from z = 40 and read the resting z. Repeat
  with a 1-unit trigger_multiple slab on the floor (OnStartTouch ->
  point_servercommand `say`): does a player resting 1.6 above the floor
  set it off? Compare with mashup's movecmp run of the same.
- B, touch order: two trigger_teleports filling one volume, A (lower
  entity index) to `dest_a`, B to `dest_b`; `setpos` into the volume,
  read `getpos` (which destination). Again with the entity order swapped
  (recompiled), with the two volumes in different BSP leaves, with one
  filtered by name, and a trigger_push overlapping a teleport (is the
  push felt before the teleport?).
- C, static props without a collision model: a prop_static of a model
  with no `.phy`, solid "Use VPhysics"; walk into it holding +forward
  and read `getpos` (stops or passes through).

## Left, ranked by maps affected

Generic, by maps affected (counts from the sweep after the fixes):

1. **Strike-generator env_beams** (visual_entities.md 4.2); map ragdolls
   and the round 2 classes are in since round 3, detail props since
   round 4 (above).
2. **Particle operators checked against CS:S** (round 2 draws `.pcf`
   effects by interpretation; tech-debt), round 3's embers and muzzle
   flashes too, and the physics numbers (buoyancy, motors, constraint
   break limits) measured.
3. **Map mistakes** the audit lists (5 kinds of inputs CS:S entities
   don't have, 7 kinds of run-time complaints): nothing to fix on our
   side unless the game is seen to do something with them.
4. **Players stuck after a shared teleport** (minigame flows, above).
5. **Materials and textures** (sweep of the evening of 2026-10-08): 19
   textures missing over 10 maps (mostly `_rt_camera`, custom cubemaps,
   files the map's author didn't pack), 11 materials missing over 8 maps,
   1 unreadable (an empty file, mg_jacks_multigames_v1), 1 unknown shader
   (Screenspace_General, mg_creative_multigames_v8_ns), 9 models
   unreadable (4 maps). Sounds: 14 missing (7 maps; mostly not packed).
6. **Decals/overlays without a surface** (11 / 5 maps, one each mostly),
   as on the stock maps (other-maps.md #11).
7. **Server commands** maps send that we refuse: SourceMod/Mani admin
   commands (ma_say, sm_say: chat text) on 5 maps, noted once per map
   since round 2 (a server without the plugin ignores them too).

Specs for the map entities (written 2026-10-08 from the public Source SDK 2013,
implemented 2026-10-09, above):
[visual_entities.md](../../../specs/source/visual_entities.md)
(point_spotlight, env_laser, env_beam, env_spritetrail, env_lightglow,
env_steam, env_spark),
[viewcontrol_and_templates.md](../../../specs/source/viewcontrol_and_templates.md)
(point_viewcontrol, point_template, env_entity_maker),
[physics_brushes.md](../../../specs/source/physics_brushes.md)
(func_physbox and _multiplayer, phys_thruster, phys_keepupright,
prop_ragdoll),
[game_entities.md](../../../specs/source/game_entities.md)
(player_speedmod, game_ui, env_fade, game_score, env_hudhint,
env_explosion, func_wall_toggle, func_conveyor). Each lists the CS:S
differences to measure under "Open questions". Also
[particles_and_smoke.md](../../../specs/source/particles_and_smoke.md)
(info_particle_system, env_smokestack, env_particlelight, env_smoketrail)
and [physics_constraints.md](../../../specs/source/physics_constraints.md)
(phys_constraint, _ballsocket, _hinge, _slideconstraint,
_lengthconstraint, _pulleyconstraint, _ragdollconstraint, phys_spring,
phys_constraintsystem, info_constraint_anchor).

Spawns: every map has spawn points; 14 have only one team's (bhop_, kz_,
some mg_: the game puts everyone on the team that has spawns).

Visual problems and slow views, second pass (2026-10-08 evening; another
session is on surf_boreas): surf_demise's magenta floor and the two real
slow-frame costs are fixed (table above). gg_simpsons_arabtoon (311 ms),
surf_sacrifice (102 ms) and mg_creative_multigames_v8_ns (45 ms) run at
the 60 Hz cap at a quiet moment with the old build too: those sweep
numbers were the machine's load. mg_kommando runs at about 40 ms under
load 12-15 with both builds while its threads are mostly idle (waiting on
presentation or the GPU; not investigated further). mg_boatrace_scramble's
first spawn isn't black here: three runs (dev and playtest builds, 60 and
600 frames) show the lit shooting range. The black view didn't come back;
the map's point_viewcontrol (`View-Startup`, handled since 2026-10-09) is the lead if it
does. The new sweep's frame times (`target/mapsweep/after/report.md` in
this session's worktree) were taken under load 8-15, so most maps sit
between the 60 Hz cap and 30 ms.
