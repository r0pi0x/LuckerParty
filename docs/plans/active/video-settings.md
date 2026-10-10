# Plan: Video > Advanced and the Audio tab's settings, one by one

Status: started 2026-10-10. Done: antialiasing, filtering, texture
detail, wait for vsync, water detail, HDR, field of view (Video >
Advanced); brightness (`mat_monitorgamma`, the video tab's gamma
dialog); the Audio tab's drop-downs filled and saved. The rest stay
greyed in the dialog until their row below is done (the greyed list:
docs/plans/active/ui-parity.md).

Each control in CS:S's `OptionsSubVideoAdvancedDlg.res` sets one or two
cvars when the dialog's OK (or Apply) is pressed; ours set them at once
(Cancel puts them back), as the rest of our options do. The cvar names
are what CS:S's console lists; the values each drop-down entry writes
were taken from the options' behaviour and want a reference capture
(below) where marked "to confirm".

## Per control

| Control | CS:S's cvars (default) | What it means in our renderer | Cost | Tests | Status |
|---|---|---|---|---|---|
| Model detail (`ModelDetail`: Low, Medium, High) | `r_rootlod` 2 / 1 / 0 (0) | Models draw their `.vtx` LOD 0 only. Low/Medium would load LOD 2 / 1 as the root (the `.vtx` has them; the MDL reader keeps only LOD 0) and draw those at every distance. Also CS:S's distance LOD switching (`r_lod`, -1: by screen size) we don't do. | Medium: the MDL/VTX reader keeps the other LODs' strip groups; the prop builder picks one. Applies on map load. | MDL test: a model's LOD 1 has fewer triangles; a map loaded at `r_rootlod 1` draws them | Greyed |
| Texture detail (`TextureDetail`: Low, Medium, High, Very High) | `mat_picmip` 2 / 1 / 0 / -1 (0) | Each map texture's sampler starts `mat_picmip` levels down its mip chain (`lod_min_clamp`): Low and Medium look as CS:S's smaller textures. -1 is as 0 (no larger textures). The full texture still loads (no memory saved). | Done (cheap). Applies from the next map load (`client::options::TextureSettings`). | `client::options` unit test (sampler clamp), menu test (the drop-down writes the cvar) | Done 2026-10-10 |
| Shader detail (`ShaderDetail`: Low, High) | `mat_reducefillrate` 1 / 0 (0) | One shader path. Low in Source drops some per-pixel work (detail textures, some env maps, phong on low-end paths: to spec). | Medium: spec first (which features `mat_reducefillrate` drops), then a material key. | Shader spec test per feature | Greyed; needs a spec |
| Water detail (`WaterDetail`) | `r_waterforceexpensive`, `r_waterforcereflectentities` (1 0) | Simple reflections, reflect world, reflect all (`map::water`). | Done | menu test `water_detail_sets_both_water_cvars` | Done |
| Shadow detail (`ShadowDetail`: Low, Medium, High) | `r_shadowrendertotexture` 0 / 1 / 1, `r_flashlightdepthtexture` 0 / 0 / 1 (to confirm) | We draw render-to-texture prop shadows (`map::shadows`, spec shadows_sky.md) = Medium. Low would draw Source's blob shadows (a round dark decal under each prop); High adds flashlight depth shadows, which CS:S has no flashlights for. | Low: a blob shadow material and a switch in `map::shadows`. High = Medium for CS:S. | Shadow spec tests for the blob path | Greyed |
| Colour correction (`ColorCorrection`: Disabled, Enabled) | `mat_colorcorrection` 0 / 1 (1, to confirm) | No colour correction: maps' `color_correction` entities and their `.raw` lookup files are ignored. Implementing means a 32x32x32 LUT post-process (Bevy has no 3D-LUT grading; a fullscreen pass like `client::gamma`) blended by the entities' weights and fade distances. | Medium to large: entity logic (map area), a `.raw` reader, the pass. Stock CS:S maps don't use it; some community maps do. | Spec for the entity's blending; a LUT unit test; a refcmp of a map with one | Greyed; needs a spec |
| Antialiasing (`AntialiasingMode`) | `mat_antialias` (0, 2, 4, 6, 8 ...), `mat_aaquality` (0; CSAA modes) | Bevy's MSAA on every camera (`client::options::apply_msaa`): None, 2x, 4x. 8x isn't supported on every GPU; CSAA modes are NVIDIA-only D3D9 and have no wgpu equivalent. FXAA/SMAA (Bevy has both) aren't CS:S choices: Lucker Party Options later if wanted. | Done | `antialias_levels` | Done |
| Filtering (`FilteringMode`: Bilinear, Trilinear, Anisotropic 2X-16X) | `mat_trilinear`, `mat_forceaniso` (0 1: bilinear, CS:S's own, read over RCON); anisotropic as `0 N` (to confirm) | Each map texture's sampler: mip filter nearest (bilinear) or linear (trilinear), anisotropy clamp N (the GPU needs every filter linear for it, so anisotropic is also trilinear). | Done (cheap). Applies from the next map load. | `client::options` unit test | Done 2026-10-10 |
| Wait for vsync (`VSync`) | `mat_vsync` (1) | The window's present mode (AutoVsync / AutoNoVsync). | Done | menu and options tests | Done |
| Motion blur (`MotionBlur`: Disabled, Enabled) | `mat_motion_blur_enabled` (0, to confirm) | None. Source's is a screen blur from the camera's turn and fall speed (not per object). Bevy's `MotionBlur` is per-object from motion vectors (needs a prepass, costs a full-screen pass); the camera-driven look needs our own pass. | Medium: a spec for Source's strength curve, then a fullscreen pass. Off by default, so low priority. | Spec test of the blur amount for a turn rate | Greyed; needs a spec |
| Multicore rendering (`Multicore`: Disabled, Enabled) | `mat_queue_mode` -1 (auto) / 0 | Bevy always renders on its own thread (pipelined rendering); there's nothing to turn off without a restart and a different app build. | None: stays greyed, a deliberate difference | - | Greyed (deliberate) |
| HDR (`HDR`, shown when the mod has HDR) | `mat_hdr_level` 0 / 1 / 2 (CS:S: 2 where the GPU can) | `client::hdr`: none, bloom, full. Applies from the next map load. | Done | `map_hdr` heavy tests | Done |
| Field of view (`FovSlider`) | `fov_desired` 75-90 (90) | The world camera unzoomed. | Done | `fov_desired_sets_the_cameras_field_of_view` | Done |
| Brightness (video tab's `GammaButton` -> `OptionsSubVideoGammaDlg.res`) | `mat_monitorgamma` 1.6-2.6 (2.2) | `client::gamma`: the finished frame (world, HUD, menus) raised to `mat_monitorgamma / 2.2` in a pass on the UI camera; no pass at 2.2. The dialog draws the install's `materials/vgui/gamma` test picture. CS:S only enables the button in full screen (hardware gamma ramp); ours works windowed too. | Done | `client::gamma` test, menu test (the dialog) | Done 2026-10-10 |

## Audio tab

| Control | CS:S's cvars (default) | Ours | Next |
|---|---|---|---|
| Speaker configuration (`SpeakerSetup`) | `snd_surround_speakers` 0 headphones, 2, 4, 5 (5.1), 7 (7.1) (CS:S: detected from the system; ours 2) | Kept and saved; the mixer (Bevy audio) is stereo, so every choice plays the stereo mix. | Headphones vs speakers: Source pans differently for each (a spec of `snd_surround_speakers`' effect on spatialisation, then `map::live_sound`'s pan law). Surround: needs a multi-channel output, not in Bevy's audio. |
| Sound quality (`SoundQuality`: Low, Medium, High) | `snd_pitchquality` + `dsp_slow_cpu`: High 1 0, Medium 0 0, Low 0 1 (High; to confirm) | Kept and saved. | `snd_pitchquality`: the resampler's interpolation when a sound plays pitched (rodio's is fixed); `dsp_slow_cpu`: cheaper room effects in `map::room`. Both need a spec of what changes. |
| Captioning (`CloseCaptionCheck`: No captions, Subtitles, Closed Captions) | `closecaption` + `cc_subtitles`: 0 0, 1 1, 1 0 (No captions) | Kept and saved; no captions drawn. | The install has `hl2/resource/closecaption_<language>.dat` (compiled) and `.txt`. CS:S's own sounds have almost no caption tokens (the file is HL2's: weapons, characters); community maps that play HL2 sounds would show them. Needs a spec (which sounds emit which token, the caption panel's layout, timing, `<clr>`, `<sfx>`, `<norepeat>` tags, `cc_subtitles` filtering sound effects out) written in a spec session, then a caption panel in `client`. |
| Audio (spoken) language (`AudioSpokenLanguage`) | Steam's language, not a cvar; needs a restart | `mashup_spoken_language`, English only: mashup mounts the install's English voice files (`hl2_sound_vo_english`). | Other languages when an install has their `*_sound_vo_<language>` files and the mount reads them. |

## Order

1. Shadow detail Low (blob shadows): small, visible on every map.
2. Model detail: the MDL reader's LODs (helps low-end playtesters).
3. Colour correction: with a spec, for the community maps that use it.
4. Shader detail and motion blur: with specs.
5. Captions: spec session first.

## Reference captures needed

From CS:S (`mat_*` printed in its console after picking each entry and
pressing Apply): Texture detail's four `mat_picmip` values; Filtering's
`mat_trilinear` and `mat_forceaniso` per entry; Shadow detail's cvars;
`mat_colorcorrection`'s and `mat_motion_blur_enabled`'s defaults;
Sound quality's two cvars per entry; `snd_surround_speakers`' value on a
stereo system. A de_dust2 view (`setpos -295 1078 120; setang 0 0 0`)
at `mat_monitorgamma` 1.6 and 2.6 in full screen, to compare the
brightness curve.
