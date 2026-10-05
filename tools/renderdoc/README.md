# RenderDoc captures of CS:S

Frame captures of the real game, to read what CS:S's shaders actually do:
constants, texture view formats (sRGB or not), samplers, per-pixel shader
traces. Our own code only; captures are game imagery and stay outside the
repo.

CS:S on Linux renders Direct3D 9 through DXVK (Vulkan). RenderDoc loads as
its implicit Vulkan layer; `rdtrigger.c` is a small preloaded library that
triggers a capture when a file appears, so nothing needs a keyboard or a
visible window.

## Capture

```
gcc -shared -fPIC -O2 -o librdtrigger.so tools/renderdoc/rdtrigger.c -ldl -lpthread
cd ~/games/counter-strike-source      # a path without spaces
SDL_VIDEODRIVER=x11 DISPLAY=:0 XDG_RUNTIME_DIR=/run/user/1000 \
SteamAppId=240 SteamGameId=240 LD_LIBRARY_PATH="$PWD/bin/linux64" \
ENABLE_VULKAN_RENDERDOC_CAPTURE=1 MASHUP_RDC_TEMPLATE=/tmp/rdc/css \
MASHUP_RDC_TRIGGER=/tmp/rdc/TRIGGER LD_PRELOAD=/path/to/librdtrigger.so \
./cstrike_linux64 -game cstrike -steam -novid -windowed -w 1280 -h 720 \
  -usercon +ip 127.0.0.1 +rcon_password mashup-refcmp +map de_dust2
# position a view (refcmp capture-ref --only <view> --keep-running reuses it)
touch /tmp/rdc/TRIGGER    # -> /tmp/rdc/css_frame<N>.rdc
```

Gotchas found getting here:
- Launch the binary directly, not `cstrike.sh`: RenderDoc strips itself
  from child processes' environments.
- Don't preload `librenderdoc.so` itself (it breaks the game's library
  lookup); the Vulkan layer loads it.
- RenderDoc's Vulkan layer has no Wayland surface support: use X11
  (`SDL_VIDEODRIVER=x11`, XWayland).
- Steam won't start the game while the account plays on another PC, and
  waits on a "pending cloud sessions" dialog after that.

## Analyse

The Python API is only inside `qrenderdoc`; run scripts headless:

```
RD_CAPTURE=<file.rdc> RD_OUT=out.txt QT_QPA_PLATFORM=offscreen qrenderdoc --python tools/renderdoc/analyse.py
```

`qrenderdoc` must have a valid `~/.local/share/qrenderdoc/UI.config`
(`{"rdocConfigData": 1, "Analytics_TotalOptOut": true, "CheckUpdate_AllowChecks": false, "Tips_HasSeenFirst": true}`)
or it stops at first-run prompts.

- `analyse.py`: follows full-screen copies back to the draw that shaded a
  pixel (RD_X, RD_Y; RD_EID to pick), dumps its constants, textures and
  shader disassembly.
- `views.py`: texture/target view formats, samplers, blend (RD_EIDS).
- `debugpx.py`: per-step shader debugger trace of one pixel (RD_EID, RD_X,
  RD_Y): sampled values, intermediate and final colour.
- `savetex.py`: saves the 1024x512 textures (lightmap atlases) as PNG.
