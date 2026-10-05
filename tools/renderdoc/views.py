import renderdoc as rd, os, traceback
out = open(os.environ["RD_OUT"], "w")
def w(*a): out.write(" ".join(str(x) for x in a) + "\n"); out.flush()
try:
    cap = rd.OpenCaptureFile(); cap.OpenFile(os.environ["RD_CAPTURE"], '', None)
    _, ctl = cap.OpenCapture(rd.ReplayOptions(), None)
    for eid in [int(e) for e in os.environ["RD_EIDS"].split(",")]:
        ctl.SetFrameEvent(eid, True)
        st = ctl.GetPipelineState()
        w("draw", eid)
        for r in st.GetReadOnlyResources(rd.ShaderStage.Pixel):
            d = r.descriptor
            if d.resource != rd.ResourceId.Null():
                w("  read", d.resource, "view format", d.format.Name())
        for s in st.GetSamplers(rd.ShaderStage.Pixel):
            sd = s.sampler
            w("  sampler filter", sd.filter.minify, sd.filter.magify, sd.filter.mip, "aniso", sd.maxAnisotropy)
        for o in st.GetOutputTargets():
            if o.resource != rd.ResourceId.Null():
                w("  write", o.resource, "view format", o.format.Name())
        bs = st.GetColorBlends()
        if bs:
            b = bs[0]
            w("  blend", b.enabled, b.colorBlend.source, b.colorBlend.destination, b.colorBlend.operation)
    ctl.Shutdown(); cap.Shutdown()
except Exception:
    w(traceback.format_exc())
out.close(); os._exit(0)
