# UI parity with CS:S's GameUI / VGUI

Goal: the menus' dialogs look and behave as CS:S's do. Their controls
are the install's `.res` layouts read at run time (never committed),
drawn with one shared widget layer (`src/client/widgets.rs`) that
behaves as VGUI's controls do; anything Lucker Party adds beyond CS:S
stays out of CS:S's dialogs (our own dialog, Lucker Party Options, or an
entry marked as ours).

Status: the widget layer, movable frames, the options dialog (all six
tabs, Voice greyed), its three Advanced dialogs (Multiplayer > Advanced
from the install's `cfg/user.scr`), the cvars behind its controls
(second pass, 2026-10), Create Server, Find Servers (filters, Add Server,
password), the loading dialog and the first-run dialog are done (first
pass, 2026-10); the brightness dialog, the Audio tab's drop-downs, Video >
Advanced's texture detail and filtering, frames hiding the menu under
them and the tabs kept on the sheet (third pass, 2026-10-10). What's
left is at the end.

## Widget layer (`client::widgets`)

State machines the dialogs' models hold (unit-tested in
`client::widgets`, the dialogs driven by scripted input in
`tests/it/menus.rs`):

| VGUI | Ours | Behaviour |
|---|---|---|
| `Frame` | `Windows`, `frame`, `VguiFrame`, `frames_pointer`, `place_frames`, `FrameBacking` | Over the game menu a frame hides the menu under it (its title and entries never show through the scheme's see-through frame colour: the frame is drawn over the part of the main menu's picture under it, or in a game over the world drawn again at 40 % (`WorldPicture`: the 3D view drawn into a picture while a dialog is open; black for the frame before it is ready); user requests 2026-10-10); Drag by the title bar (26 scheme px; not the close box), kept wholly on screen; a press anywhere on a frame brings it to the front (`GlobalZIndex` by stacking rank, modal dialogs over their owner); close box (X) = the dialog's Cancel; sizeable frames (the server browser) resize from their edges (5 px) and bottom-right corner (18 px, the grip drawn), not under their minimum; centred the first time it opens in a session, then where it was left; nothing saved across runs |
| `ComboBox` | `ComboList`, `combo_box`, `combo_popup` | Click (or Space) opens the list under the box, the arrow button sunken while open; the pointer highlights an entry, a click picks it; Up/Down/PageUp/PageDown/Home/End, Enter picks, Esc or a click outside closes (the click is taken); the wheel scrolls the open list; closed and focused, Up/Down and the wheel step it (no wrapping) and a letter jumps; more than 10 entries scroll with a scroll bar; focused, its text shows selected |
| `Slider` / `CCvarSlider` | `SliderDrag`, `SliderTrack`, `slider` | Press anywhere on the track and drag: the value follows the pointer while held (even as the dialog redraws); 11 tick marks under it; Left/Right (and Up/Down) step it when focused; `leftText` / `rightText` (Low / High) under its ends |
| `TextEntry` | `Caret`, `text_entry`, `caret_from_click` | Caret drawn and kept in view (long text scrolls); click places the caret at the nearest char (`UiFonts::char_offsets` measures the face); Shift+arrows/Home/End select, Ctrl+A, Ctrl+C, Ctrl+X, Ctrl+V; typing replaces the selection; Tab to it selects all; numeric entries take digits only, `maxchars` kept; passwords show `*` and never copy |
| `CheckButton` / `CCvarToggleCheckButton` / `CCvarNegateCheckButton` | `check_button` | Click on the box or its words, Space when focused |
| `RadioButton` | `radio_button` | Round box; a click picks it in its group |
| `Button` | `button`, `ButtonStyle`, `style_buttons` | Armed (hover) and depressed colours from the scheme (`Button.Armed*`, `Button.Depressed*`), sunken while held, disabled text engraved and no hit; the default button and the focused one ringed; a `ToggleButton` (Filters) stays sunken while on |
| `PropertySheet` tabs | `tabs`, `tab_widths` | Each as wide as its words (plus 12 a side, at least 56), shrunk in proportion if they would pass the sheet's edge (the options' Multiplayer tab stuck out at 96 each); the open tab raised and joined to its page; Ctrl+Tab / Ctrl+Shift+Tab step them |
| Tab order | `focus_step` | Tab / Shift+Tab walk the dialog's controls (the `.res` order); Enter presses the focused button or list row, else the dialog's default button (OK, Start, Connect); Space presses the focused control; Esc cancels |
| `ListPanel` | `server_browser::server_list` | Sort by a header click, the sort arrow drawn at its right; drag a header's right edge to size the column; selected row in the scheme's colours; wheel; double-click |

## Audit: dialogs and their controls

Classes are what the install's `.res` file declares (CS:S Steam build,
`hl2/resource/`, `cstrike/resource/`, `platform/servers/`). "Before"
is the main branch before this work; "Now" after.

### Options (`OptionsDialog`: code; pages `OptionsSub*.res`)

Before: a centred, fixed frame; each tab a column of our own rows in
our order (label, then `<` value `>` steppers for choices, a slider
without ticks, a check box); one OK, "changes apply at once". Now: 512 x
406 frame with OK / Cancel / Apply (changes still apply at once so they
can be seen, Cancel puts back what changed since the dialog opened or
since Apply, Apply is greyed with nothing to apply); each page drawn
from its `.res`, controls mashup lacks greyed.

| Page | Control (`fieldName`) | `.res` class | Now |
|---|---|---|---|
| Keyboard | `listpanel_keybindlist` | SectionedListPanel | the action list (as before) |
| | `Defaults`, `KeyAdvancedButton`, `ChangeKeyButton`, `ClearKeyButton` | Button | widget buttons where the file puts them |
| Keyboard > Advanced | `FastSwitchCheck`, `ConsoleCheck` | CheckButton | `hud_fastswitch`, `con_enable`; OK (default) / Cancel; modal over the options |
| Mouse | `ReverseMouse` | CCvarNegateCheckButton | `m_pitch` negated |
| | `Slider` + `SensitivityLabel` | CCvarSlider + TextEntry | `sensitivity`: slider with ticks and Low/High; the entry shows the value and sets it when it reads as one in range |
| | `MouseFilter` | CCvarToggleCheckButton | `m_filter` (0): averages this frame's motion with the last's |
| | `MouseAccelerationCheckbox` + `MouseAccelerationSlider` + `MouseAccelerationLabel` | CheckButton / CCvarSlider / TextEntry | `m_customaccel` (0; ticked: 3), `m_customaccel_exponent` (1.05; slider 1 to 1.4, the entry beside it) |
| | `MouseRaw`, `Joystick*` | CCvarToggleCheckButton | greyed (see below); those whose words the install lacks are left out |
| Audio | `SFXSlider` | CCvarSlider | `volume` |
| | `MusicSlider` | CCvarSlider | `snd_musicvolume` (1): sounds under `music/` and MP3s (`map::live_sound::is_music`) |
| | `snd_mute_losefocus` | CCvarToggleCheckButton | `snd_mute_losefocus` (1): silent while the window is in the background (`client::audio`) |
| | `SpeakerSetup` | ComboBox | `snd_surround_speakers` (ours 2): Headphones (0), 2, 4, 5.1, 7.1 Speakers; kept and saved, the mixer stays stereo (docs/plans/active/video-settings.md, "Audio tab") |
| | `SoundQuality` | ComboBox | `snd_pitchquality` and `dsp_slow_cpu` together: Low (0 1), Medium (0 0), High (1 0, the default); kept and saved, one mixer quality |
| | `CloseCaptionCheck` | ComboBox | `closecaption` and `cc_subtitles` together: No captions (0 0, the default), Subtitles (1 1), Closed Captions (1 0); kept and saved, captions not drawn yet (needs a spec) |
| | `AudioSpokenLanguage` | ComboBox | `mashup_spoken_language` (CS:S's is Steam's language, not a cvar): English, the voice files mashup mounts |
| Video | `Resolution` | ComboBox | `mashup_resolution`: the monitor's modes in a drop-down, those of the aspect ratio picked |
| | `AspectRatio` | ComboBox | Normal (4:3), Widescreen 16:9, Widescreen 16:10: filters the resolutions (from the size now; picking one of another ratio moves the size to that ratio's largest, as CS:S's list refills); no cvar |
| | `DisplayModeCombo` | ComboBox | `mashup_fullscreen`: Full screen, Windowed (and Borderless) |
| | `AdvancedButton` | Button | opens Video > Advanced |
| | `GammaButton` | Button | opens the brightness dialog (below); the file has it disabled, CS:S enables it in code (in full screen; to confirm) |
| | `VRMode`, `VRModeLabel` | ComboBox / Label | left out (a deliberate difference, below) |
| Brightness (`OptionsSubVideoGammaDlg.res`, 290 x 396) | `Gamma` + `GammaEntry` | CCvarSlider (LIGHT / DARK) + TextEntry | `mat_monitorgamma` (2.2; 1.6 to 2.6): the whole frame raised to `mat_monitorgamma / 2.2` (`client::gamma`); the entry shows and sets it |
| | `ImagePanel1` | ImagePanel (`image gamma`) | the install's `materials/vgui/gamma` test picture |
| | `OKButton`, `Button1` | Button | OK (default) / Cancel (puts it back); modal over the options |
| Video > Advanced (`OptionsSubVideoAdvancedDlg.res`, 482 x 358) | `AntialiasingMode` | ComboBox | `mat_antialias` (None, 2x, 4x MSAA) |
| | `WaterDetail` | ComboBox | `r_waterforceexpensive` and `r_waterforcereflectentities` together: Simple reflections (0 0), Reflect world (1 0, the default), Reflect all (1 1) |
| | `VSync` | ComboBox | `mat_vsync` as Disabled / Enabled |
| | `HDR` (hidden in the file, shown when the mod has HDR) | ComboBox | `mat_hdr_level` |
| | `FovSlider` | CCvarSlider (`cvar_name fov_desired`, 75 to 90) | `fov_desired` (90): the world camera unzoomed (`options::PlayerFov`, `client::zoom_camera`); the view model zooms with it as with a scope (`viewmodel_fov` minus 90 minus it); a scope's zoom is its own; zoomed mouse scaling still divides by 90 |
| | `ModelDetail` | ComboBox | `r_rootlod`: Low (2), Medium (1), High (0, the default): the finest level of detail props draw; they switch to coarser ones with distance (`map::lod`); at once |
| | `ShadowDetail` | ComboBox | `r_shadowrendertotexture` and `r_flashlightdepthtexture` together: Low (0 0: blob shadows), Medium (1 0, the default: render-to-texture), High (1 1, as Medium: no flashlights); at once (`map::shadows`) |
| | `TextureDetail` | ComboBox | `mat_picmip`: Low (2), Medium (1), High (0, the default), Very High (-1, as High): each map texture starts that many levels down its mips (`client::options::TextureSettings`), from the next map |
| | `FilteringMode` | ComboBox | `mat_trilinear` and `mat_forceaniso` together: Bilinear (0 1, CS:S's default), Trilinear (1 1), Anisotropic 2X-16X (0 N); from the next map |
| | `ShaderDetail`, `ColorCorrection`, `MotionBlur`, `Multicore` | ComboBox | greyed (see below; the plan: docs/plans/active/video-settings.md) |
| Voice (`OptionsSubVoice.res`; the tab was missing) | `voice_modenable`, `VoiceReceive`, `MicBoost`, `TestMicrophone`, `MicMeter` | CheckButton / CCvarSlider / Button / ImagePanel | the tab, all greyed: mashup has no voice chat |
| Multiplayer (`cstrike/resource/OptionsSubMultiplayer.res`) | `CrosshairColorComboBox` | ComboBox | `cl_crosshaircolor`: Green, Red, Blue, Yellow, Cyan, Custom (5) |
| | `Red/Green/Blue Color Slider` | CCvarSlider | `cl_crosshaircolor_r`, `_g`, `_b` (50, 250, 50): the Custom colour |
| | `Size Slider`, `Thickness Slider` | CCvarSlider | `cl_crosshairsize` (5; 0 to 10), `cl_crosshairthickness` (0.5; 0 to 3): line length and width, 5 and 0.5 our lines as they were |
| | `Alpha Slider` | CCvarSlider | `cl_crosshairalpha` |
| | `CrosshairTranslucencyCheckbox`, `CrosshairDynamicCheckbox`, `CrosshairDotCheckbox` | CCvarToggleCheckButton | `cl_crosshairusealpha`, `cl_dynamiccrosshair`, `cl_crosshairdot` (0: a centre dot as wide as the lines) |
| | `CrosshairImage` | CrosshairImagePanelCS | our crosshair preview (all of the above) |
| | `LockRadarRotationCheckbox` | CCvarToggleCheckButton | `cl_radar_locked` (0): the radar keeps the overview as drawn |
| | `DownloadFilterCheck` | ComboBox | `cl_downloadfilter` (all, nosounds, mapsonly, none): servers here send only maps, so only "none" refuses (joining then fails as a missing map) |
| | `Advanced` | Button | opens Multiplayer > Advanced |
| | `ImportSprayImage`, `ResetStats`, `LogoImage` | Button / ImagePanel | greyed (see below) |
| Multiplayer > Advanced (`MultiplayerAdvancedDialog.res`, 540 x 376; the list from the install's `cfg/user.scr`, else `cfg/user_default.scr`, read at run time) | `PanelListPanel` | CPanelListPanel | the script's options in its order, as Create Server's Game page reads `settings.scr` (`gameui::scr_settings`, one parser): BOOL a check box, LIST a drop-down, NUMBER / STRING a text entry; those mashup has: `mp_decals` (200: runtime decals drawn, the oldest first; a client cvar here as in Source), `cl_righthand`, `cl_c4progressbar` (1: the defuse bar); the rest greyed showing the script's default (`cl_clanid`, `cl_autowepswitch`, `hud_centerid`, `cl_autohelp`, `hud_takesshots`, `cl_disablefreezecam`, `cl_disablehtmlmotd`, `cl_cloud_settings`); without the install: ours, the weapon hand |
| | `OK`, `Cancel` | Button | as CS:S's dialog: changes wait for OK, which sets those changed; Cancel (Esc, the X) drops them; modal over the options |

### Spectator menu (`resource/ui/bottomspectator.res`; `client::spectator_menu`)

| Control | `.res` class | Now |
|---|---|---|
| `settingscombo` | ComboBox | "Options": the install's `resource/spectatormenu.res` command menu (Close; Settings: No Rotation, Show Names, Show Health, Show Tracks, toggling `overview_locked`/`_names`/`_health`/`_tracks`; Overview: No / Small / Large Map, Zoom In / Out; Auto Director, greyed: none in mashup; Show Scores, `togglescores`), submenus opening beside their entry |
| `playercombo`, `specprev`, `specnext` | ComboBox, Button | the players who may be watched; the watched one as "name (health)"; `<` `>` = `spec_prev` / `spec_next` |
| `viewcombo` | ComboBox | the install's `resource/spectatormodes.res`: First Person, Chase Camera, Free Look (`spec_mode 4/5/6`) |

Drawn with the shared widgets in the GameUI scheme (CS:S draws them in the
client scheme); the lists open upward. The bar's own text (the player's
name, the mode) is hidden while the menu is open.

Lucker Party Options (ours, from our main-menu entries): zoom
sensitivity ratio, room reverb (`dsp_volume`), crosshair scale, view
model FOV, show FPS. Before they were rows on CS:S's tabs. The weapon
hand (`cl_righthand`) moved to CS:S's place for it, Multiplayer >
Advanced.

Every new cvar is archived (saved in config.cfg when it differs from
its default, `host_writeconfig`) and changes the game at once; their
defaults are CS:S's (`client::options` tests
`the_options_cvars_start_as_css_and_persist_in_the_config`).

#### Greyed, and why

| Control | Why it stays greyed |
|---|---|
| Mouse `MouseRaw` (`m_rawinput`) | mashup always reads raw device motion (Bevy's mouse motion, no OS acceleration); a check box that changed nothing would mislead |
| Mouse `Joystick*` | no joystick or gamepad input |
| Audio `ThirdPartySoundCredits`, Video `ThirdPartyVideoCredits` | links out (URLButton) |
| Video > Advanced `ShaderDetail` | one shader path (docs/plans/active/video-settings.md) |
| Video > Advanced `ColorCorrection`, `MotionBlur` | the renderer has neither (no colour correction or motion blur at all; same plan) |
| Video > Advanced `Multicore` | Bevy always renders multi-threaded |
| Voice (all) | no voice chat |
| Multiplayer `ImportSprayImage`, `LogoImage` | no sprays |
| Multiplayer `ResetStats` | no stats |
| Multiplayer > Advanced `cl_clanid`, `cl_disablehtmlmotd`, `cl_cloud_settings`, `hud_takesshots` | no clans, no MOTD, no Steam Cloud, no end-of-map screenshots |
| Multiplayer > Advanced `cl_autohelp` | hints aren't in mashup yet: backlog |

#### Deliberate differences (CS:S's own, kept apart)

- `con_enable`: ours defaults to 1, CS:S's to 0 (the console opens
  without ticking "Enable developer console" first). Undecided; the
  user hasn't chosen, so it stays 1.
- `volume`: ours starts at 0.5 (`client::audio::DEFAULT_VOLUME`), CS:S's
  at 1. Undecided.
- The options dialog applies each change at once (Cancel puts them
  back); CS:S's waits for OK or Apply. Its Advanced dialogs behave the
  same way, except Multiplayer > Advanced, which waits for OK as CS:S's.
- Virtual Reality Mode (`VRMode` and its label on the video tab): left
  out (user decision 2026-10-10: mashup won't support VR; CS:S greys it
  without a headset). `game_menu::draw::LEFT_OUT` lists them.
- Brightness works windowed too (CS:S's needs full screen for its
  hardware gamma ramp; ours is a pass over the frame).
- Texture detail and filtering apply from the next map load (the
  textures live only on the GPU once drawn); CS:S's apply at once.

#### Reference captures needed

What the install's files don't say (CS:S sets it in code) and was
chosen here; a capture of the reference client would settle each:

- The Multiplayer tab's sliders' ranges: Size (here 0 to 10),
  Thickness (0 to 3), the colour sliders (0 to 255), and how size and
  thickness map to pixels (here 5 and 0.5 are our previous lines).
- The colour drop-down's entries and order (here Green, Red, Blue,
  Yellow, Cyan, Custom; whether Custom is 5) and whether the colour
  sliders grey out unless Custom is picked.
- `cl_crosshairusealpha`'s default (here 0, as before this work).
- The mouse acceleration slider's range (here 1 to 1.4), what the check
  box writes to `m_customaccel` (here 3) and the curve itself.
- `mp_decals`'s default (here 200; `user_default.scr` says 512, which
  is the script's starting value, not the cvar's).
- Whether CS:S shows the video Advanced dialog's FOV slider at all
  (the file has it visible; multiplayer mods may hide it).
- The Voice tab's layout with its meters, for the greyed look.

The options' words now come from every copy of the strings files along
the search path (CS:S merges them): cstrike's `gameui_english.txt` lacks
words hl2's has, so labels that were missing before show (the FOV
slider's, Motion Blur, Multicore Rendering, the colour sliders' Red /
Green / Blue, Virtual Reality Mode).

### Create Server (`CCreateMultiplayerGameDialog`: code; pages `.res`)

Before: one page of our own: Map, Mode, Terrorist bots, CT bots, Bot
difficulty as `<` value `>` steppers, a map thumbnail (CS:S's dialog
has none), Start and Back; a separate map list page. Now: 348 x 460
frame, a property sheet (Server, Game, Bot), Start (default) / Cancel.

| Page | Control | `.res` class | Now |
|---|---|---|---|
| Server (`cstrike/resource/CreateMultiplayerGameServerPage.res`) | `MapList` | ComboBox (not editable) | `< Random Map >` (`#GameUI_RandomMap`) first, then the maps; random plays one of them at Start |
| | `EnableBotsCheck` | CheckButton | include bots |
| | `BotQuotaCombo` | TextEntry (numeric, 2 chars) | how many (any team: split, the odd one a terrorist) |
| | `SkillLevel0..3` | RadioButton | the difficulty presets |
| | `VisibilityType` | ComboBox | greyed |
| Game (`CreateMultiplayerGameGameplayPage.res`) | `GameOptions` | CPanelListPanel from `cfg/settings.scr` | the script's options mashup has a cvar for (`hostname`, `maxplayers`, `sv_password`, `mp_*` ...): STRING/NUMBER as text entries, BOOL as check boxes, LIST as drop-downs, scrolled; then ours, "Rounds (off: deathmatch)"; Start sets those changed |
| Bot (`cstrike/resource/CreateMultiplayerGameBotPage.res`) | `BotJoinTeamCombo` | ComboBox | which team bots join (Any, Terrorists, Counter-Terrorists) |
| | `BotPrefixEntry` | TextEntry | `bot_prefix` |
| | `BotJoinAfterPlayerCheck` | CCvarToggleCheckButton | `bot_join_after_player` |
| | `BotAllow*Check`, `BotDeferToHumanCheck`, `BotChatterCombo` | CCvarToggleCheckButton / ComboBox | greyed |

### Find Servers (`platform/servers/`)

| Control | `.res` class | Before | Now |
|---|---|---|---|
| `CServerBrowserDialog` | Frame (`DialogServerBrowser.res`, 640 x 384) | fixed, centred; an "x" button | movable, sizeable (minimum 640 x 384, the grip drawn), stacked with the menu's dialogs; close box |
| `GameTabs` | PropertySheet (autoResize 3) | tabs | tabs (Internet greyed); grows with the frame |
| `gamelist` | ListPanel (autoResize 3) | headers sort (arrow in the text), fixed widths | sort arrow at the header's right, header edges size columns, grows with the frame |
| `ConnectButton`, `RefreshButton`, `RefreshQuickButton`, `AddServerButton` | Button (pinCorner 3) | drawn, no hover/pressed look | widget buttons; Connect is the default (Enter); follow the bottom-right corner |
| `Filter` | ToggleButton | pressed look while shown | latched widget button |
| `PingFilter` | ComboBox | a box whose left/right halves stepped the value (`Latency(±1)`) | a real drop-down (`< 50` ... `< 600`) |
| `GameFilter`, `LocationFilter`, `SecureFilter` | ComboBox | greyed box | greyed combo box (LAN servers have none of these) |
| `MapFilter`, `MaxPlayerFilter` | TextEntry | typed text with a `_` caret, Backspace only | caret, selection, copy/cut/paste, click to place |
| `ServerFullFilterCheck`, `ServerEmptyFilterCheck`, `NoPasswordFilterCheck` | CheckButton | check boxes | check buttons in the Tab order (Space ticks) |
| Add Server (`DialogAddServer.res`) | Frame, TextEntry, Buttons, ListPanel | centred, fixed | modal movable frame; widget controls; "Add this address" default |
| Password (`DialogServerPassword.res`) | Frame, TextEntry (hidden) | centred, fixed | modal movable frame; `*` entry; Connect default |

### Other dialogs

| Dialog | Source | Now |
|---|---|---|
| Loading (`LoadingDialogNoBanner.res`: Frame, Label, ProgressBar, Button) | install | movable frame (no close box, as CS:S's), Cancel the default button |
| Disconnected (failure) | ours, in the loading dialog's look | movable frame, Close |
| CS:S Not Found (first run) | ours | modal frame; folder text entry with caret, selection, paste; Retry default, Tab order |
| Bots, Team (in a game) | ours | frames with widget buttons (column) |
| Console | ours (a drop-down console, not VGUI) | unchanged: CS:S's console is a sizeable VGUI frame; see Left |
| Buy and team menus | the map's `resource/ui/*.res` through `client::vgui` | unchanged: full-screen VGUI panels in CS:S too, not movable |

## Screenshots

Taken at 1280 x 720 with the dev build (`--screenshot`); kept out of the
repo (game imagery). Before (main): Create Server, Options > Video,
Audio, Mouse, Multiplayer, Find Servers with the filters. After: the
same plus Create Server with the map drop-down open (`+menu newgame
+menuinput open`), Find Servers with the latency drop-down open
(`+openserverbrowser lan +serverbrowser filters +serverbrowser latency
+serverbrowser hover 2`), Video > Advanced, and the browser dragged and
resized (`+wait 30 +vgui_windows servers 40 30 760 520`). Second pass
(the cvars): each options tab (`+wait 60 +menu mouse`, `audio`,
`video`, `voice`, `multiplayer` with `+cl_crosshaircolor 5
+cl_crosshairdot 1 +cl_crosshairsize 8` before it), `videoadvanced`,
`mpadvanced`, `advanced`, Create Server's map list open (`+menu
newgame +menuinput open`: `< Random Map >` first) and `extras`. Third
pass (2026-10-10, layering and the empty controls): Options, Create
Server and Find Servers over the main menu (the title hidden under the
frames) and Options over the in-game Esc menu (`--map
cs_source:de_dust2 +wait 60 +menu options`), the tabs inside the sheet,
the Audio tab's drop-downs, the video tab without VR mode, `menu gamma`
at `mat_monitorgamma` 1.6, 2.2 and 2.6, and de_dust2 at the three.

## Left

- The console as CS:S's: a sizeable, movable VGUI frame (`CConsoleDialog`,
  no `.res`: its layout is code) instead of our drop-down.
- The greyed controls whose features mashup may get (Video > Advanced:
  docs/plans/active/video-settings.md; Multiplayer > Advanced's
  auto-help). Its `cl_autowepswitch`,
  `hud_centerid` and `cl_disablefreezecam` work (`weapon::drop`,
  `client::target_id`, `client::freeze_cam`).
- Reference captures of CS:S's dialogs to compare with (see the report
  of this work: an open combo box's list colours, the slider's tick
  count, a focused control's ring).
