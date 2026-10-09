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

## Left, ranked by maps affected

Generic, by maps affected (counts from the sweep after the fixes):

1. **Visual entities not drawn**: env_spritetrail (8 maps),
   info_particle_system (7), env_smokestack (3), strike env_beams;
   prop_ragdoll (6) isn't placed (physics_brushes.md 6).
2. **Unspecced classes**: logic_measure_movement (3 maps, 39),
   func_rot_button, momentary_rot_button, phys_motor, phys_constraint/
   ballsocket, point_teleport, point_push, env_texturetoggle,
   env_screenoverlay, func_water_analog.
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
differences to measure under "Open questions". Not yet specced:
info_particle_system, env_smokestack, phys_constraint/ballsocket.

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
