# UI parity with CS:S's GameUI / VGUI

Goal: the menus' dialogs look and behave as CS:S's do. Their controls
are the install's `.res` layouts read at run time (never committed),
drawn with one shared widget layer (`src/client/widgets.rs`) that
behaves as VGUI's controls do; anything Lucker Party adds beyond CS:S
stays out of CS:S's dialogs (our own dialog, Lucker Party Options, or an
entry marked as ours).

Status: the widget layer, movable frames, the options dialog, its two
Advanced dialogs, Create Server, Find Servers (filters, Add Server,
password), the loading dialog and the first-run dialog are done (first
pass, 2026-10). What's left is at the end.

## Widget layer (`client::widgets`)

State machines the dialogs' models hold (unit-tested in
`client::widgets`, the dialogs driven by scripted input in
`tests/it/menus.rs`):

| VGUI | Ours | Behaviour |
|---|---|---|
| `Frame` | `Windows`, `frame`, `VguiFrame`, `frames_pointer`, `place_frames` | Drag by the title bar (26 scheme px; not the close box), kept wholly on screen; a press anywhere on a frame brings it to the front (`GlobalZIndex` by stacking rank, modal dialogs over their owner); close box (X) = the dialog's Cancel; sizeable frames (the server browser) resize from their edges (5 px) and bottom-right corner (18 px, the grip drawn), not under their minimum; centred the first time it opens in a session, then where it was left; nothing saved across runs |
| `ComboBox` | `ComboList`, `combo_box`, `combo_popup` | Click (or Space) opens the list under the box, the arrow button sunken while open; the pointer highlights an entry, a click picks it; Up/Down/PageUp/PageDown/Home/End, Enter picks, Esc or a click outside closes (the click is taken); the wheel scrolls the open list; closed and focused, Up/Down and the wheel step it (no wrapping) and a letter jumps; more than 10 entries scroll with a scroll bar; focused, its text shows selected |
| `Slider` / `CCvarSlider` | `SliderDrag`, `SliderTrack`, `slider` | Press anywhere on the track and drag: the value follows the pointer while held (even as the dialog redraws); 11 tick marks under it; Left/Right (and Up/Down) step it when focused; `leftText` / `rightText` (Low / High) under its ends |
| `TextEntry` | `Caret`, `text_entry`, `caret_from_click` | Caret drawn and kept in view (long text scrolls); click places the caret at the nearest char (`UiFonts::char_offsets` measures the face); Shift+arrows/Home/End select, Ctrl+A, Ctrl+C, Ctrl+X, Ctrl+V; typing replaces the selection; Tab to it selects all; numeric entries take digits only, `maxchars` kept; passwords show `*` and never copy |
| `CheckButton` / `CCvarToggleCheckButton` / `CCvarNegateCheckButton` | `check_button` | Click on the box or its words, Space when focused |
| `RadioButton` | `radio_button` | Round box; a click picks it in its group |
| `Button` | `button`, `ButtonStyle`, `style_buttons` | Armed (hover) and depressed colours from the scheme (`Button.Armed*`, `Button.Depressed*`), sunken while held, disabled text engraved and no hit; the default button and the focused one ringed; a `ToggleButton` (Filters) stays sunken while on |
| `PropertySheet` tabs | `tabs` | The open tab raised and joined to its page; Ctrl+Tab / Ctrl+Shift+Tab step them |
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
| Keyboard > Advanced | `FastSwitchCheck`, `ConsoleCheck` | CheckButton | check buttons; OK (default) / Cancel; modal over the options |
| Mouse | `ReverseMouse` | CCvarNegateCheckButton | `m_pitch` negated |
| | `Slider` + `SensitivityLabel` | CCvarSlider + TextEntry | `sensitivity`: slider with ticks and Low/High; the entry shows the value and sets it when it reads as one in range |
| | `MouseFilter`, `MouseRaw`, `Joystick*`, `MouseAcceleration*` | CCvarToggleCheckButton / CheckButton / CCvarSlider / TextEntry | greyed (mashup lacks `m_filter`, `m_rawinput`, joysticks); those whose words the install lacks are left out |
| Audio | `SFXSlider` | CCvarSlider | `volume` |
| | `MusicSlider`, `snd_mute_losefocus` | CCvarSlider / CCvarToggleCheckButton | greyed (no cvar) |
| | `SpeakerSetup`, `SoundQuality`, `CloseCaptionCheck`, `AudioSpokenLanguage` | ComboBox | greyed |
| Video | `Resolution` | ComboBox | `mashup_resolution`: the monitor's modes in a drop-down |
| | `DisplayModeCombo` | ComboBox | `mashup_fullscreen`: Full screen, Windowed (and Borderless) |
| | `AdvancedButton` | Button | opens Video > Advanced |
| | `AspectRatio`, `GammaButton` | ComboBox / Button | greyed; `VRMode` (no words in the install) left out |
| Video > Advanced (`OptionsSubVideoAdvancedDlg.res`, 482 x 358) | `AntialiasingMode` | ComboBox | `mat_antialias` (None, 2x, 4x MSAA) |
| | `VSync` | ComboBox | `mat_vsync` as Disabled / Enabled |
| | `HDR` (hidden in the file, shown when the mod has HDR) | ComboBox | `mat_hdr_level` |
| | `FovSlider` | CCvarSlider (`cvar_name fov_desired`) | greyed (no `fov_desired`) |
| | `ModelDetail`, `TextureDetail`, `ShaderDetail`, `WaterDetail`, `ShadowDetail`, `ColorCorrection`, `FilteringMode`, `MotionBlur`, `Multicore` | ComboBox | greyed |
| Multiplayer (`cstrike/resource/OptionsSubMultiplayer.res`) | `CrosshairColorComboBox` | ComboBox | `cl_crosshaircolor` |
| | `Alpha Slider` | CCvarSlider | `cl_crosshairalpha` |
| | `CrosshairTranslucencyCheckbox`, `CrosshairDynamicCheckbox` | CCvarToggleCheckButton | `cl_crosshairusealpha`, `cl_dynamiccrosshair` |
| | `CrosshairImage` | CrosshairImagePanelCS | our crosshair preview |
| | `Size Slider`, `Thickness Slider`, `Red/Green/Blue Color Slider`, `CrosshairDotCheckbox`, `LockRadarRotationCheckbox`, `DownloadFilterCheck` | CCvarSlider / CCvarToggleCheckButton / ComboBox | greyed (mashup's crosshair has `cl_crosshairscale` instead: in Lucker Party Options) |
| | `ImportSprayImage`, `Advanced`, `ResetStats`, `LogoImage` | Button / ImagePanel | greyed |

Lucker Party Options (ours, from our main-menu entries): zoom
sensitivity ratio, room reverb (`dsp_volume`), crosshair scale, weapon
hand (`cl_righthand`), view model FOV, show FPS. Before they were rows
on CS:S's tabs.

### Create Server (`CCreateMultiplayerGameDialog`: code; pages `.res`)

Before: one page of our own: Map, Mode, Terrorist bots, CT bots, Bot
difficulty as `<` value `>` steppers, a map thumbnail (CS:S's dialog
has none), Start and Back; a separate map list page. Now: 348 x 460
frame, a property sheet (Server, Game, Bot), Start (default) / Cancel.

| Page | Control | `.res` class | Now |
|---|---|---|---|
| Server (`cstrike/resource/CreateMultiplayerGameServerPage.res`) | `MapList` | ComboBox (not editable) | the maps in a drop-down |
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
resized (`+wait 30 +vgui_windows servers 40 30 760 520`).

## Left

- The console as CS:S's: a sizeable, movable VGUI frame (`CConsoleDialog`,
  no `.res`: its layout is code) instead of our drop-down.
- Multiplayer > Advanced (`cfg/user.scr`, the same script format as
  `settings.scr`: a list of the client's own options).
- Cvars for greyed controls that matter (`snd_musicvolume`,
  `fov_desired`, `cl_crosshairsize`/`thickness`/`dot`, `m_rawinput`).
- Reference captures of CS:S's dialogs to compare with (see the report
  of this work: an open combo box's list colours, the slider's tick
  count, a focused control's ring).
- A Create Server "< Random Map >" entry (`#GameUI_RandomMap`).
