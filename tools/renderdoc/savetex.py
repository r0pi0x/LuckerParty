import renderdoc as rd, os, traceback
out = open(os.environ["RD_OUT"], "w")
try:
    cap = rd.OpenCaptureFile(); cap.OpenFile(os.environ["RD_CAPTURE"], '', None)
    _, ctl = cap.OpenCapture(rd.ReplayOptions(), None)
    ctl.SetFrameEvent(531, True)
    for t in ctl.GetTextures():
        if t.width == 1024 and t.height == 512:
            s = rd.TextureSave()
            s.resourceId = t.resourceId
            s.destType = rd.FileType.PNG
            s.mip = 0
            path = os.environ["RD_DIR"] + f"/lightmap_{int(t.resourceId)}.png"
            ctl.SaveTexture(s, path)
            out.write(f"saved {path} {t.format.Name()}\n")
    ctl.Shutdown(); cap.Shutdown()
except Exception:
    out.write(traceback.format_exc())
out.close(); os._exit(0)
