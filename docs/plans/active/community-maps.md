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
5. Round 2's (noted, not done here): crazykart's karts (players
   parented), boatrace's and creative's boats (buoyancy), item powers'
   visuals (spritetrails, particles), `mp_flashlight`/`sv_alltalk`
   unknown settings (logged).

## Left, ranked by maps affected

Generic, by maps affected (counts from the sweep after the fixes):

1. **Visual entities not drawn**: env_spritetrail (8 maps),
   info_particle_system (7), env_smokestack (3), strike env_beams;
   prop_ragdoll (6) isn't placed (physics_brushes.md 6).
2. **Unspecced classes** (logic_measure_movement, func_rot_button,
   momentary_rot_button, point_teleport, logic_multicompare and env_shake
   are in since the map logic audit, above): phys_motor,
   phys_constraint/ballsocket, point_push, env_texturetoggle,
   env_screenoverlay, func_water_analog, func_monitor/point_camera,
   point_tesla, env_shooter.
3. **env_tonemap_controller inputs** (15 maps: SetBloomScale,
   SetAutoExposureMin/Max): HDR look, logic logs them unhandled.
4. **Triggers whose filtername names a missing filter** (5 maps, 341
   triggers: filter_blue/filter_red on surf_ maps): the game then lets
   every activator through, as we do; only the log line is noise.
5. **Materials and textures** (sweep of the evening of 2026-10-08): 19
   textures missing over 10 maps (mostly `_rt_camera`, custom cubemaps,
   files the map's author didn't pack), 11 materials missing over 8 maps,
   1 unreadable (an empty file, mg_jacks_multigames_v1), 1 unknown shader
   (Screenspace_General, mg_creative_multigames_v8_ns), 9 models
   unreadable (4 maps). Sounds: 14 missing (7 maps; mostly not packed).
6. **Decals/overlays without a surface** (11 / 5 maps, one each mostly),
   as on the stock maps (other-maps.md #11).
7. **Server commands** maps send that we refuse: SourceMod/Mani admin
   commands (ma_say, sm_say: chat text) on 4 maps.

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
