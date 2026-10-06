"""Pixel history of one pixel of the frame's final colour target: every
draw that touched it, whether it passed the depth test, and the colour
before and after (RD_X, RD_Y; RD_EID: the event whose bound colour target
to inspect, default the last draw).
"""
import renderdoc as rd, os, traceback

out = open(os.environ["RD_OUT"], "w")


def w(*a):
    out.write(" ".join(str(x) for x in a) + "\n")
    out.flush()


def actions(roots):
    for a in roots:
        yield a
        yield from actions(a.children)


try:
    cap = rd.OpenCaptureFile()
    cap.OpenFile(os.environ["RD_CAPTURE"], "", None)
    _, ctl = cap.OpenCapture(rd.ReplayOptions(), None)
    eid = int(os.environ.get("RD_EID", "0")) or max(
        a.eventId for a in actions(ctl.GetRootActions()) if a.flags & rd.ActionFlags.Drawcall
    )
    ctl.SetFrameEvent(eid, True)
    target = [o for o in ctl.GetPipelineState().GetOutputTargets() if o.resource != rd.ResourceId.Null()][0]
    x, y = int(os.environ["RD_X"]), int(os.environ["RD_Y"])
    w("target", target.resource, "at event", eid)
    sub = rd.Subresource(0, 0, 0)
    for m in ctl.PixelHistory(target.resource, x, y, sub, rd.CompType.Typeless):
        pre = [round(m.preMod.col.floatValue[k], 4) for k in range(4)]
        post = [round(m.postMod.col.floatValue[k], 4) for k in range(4)]
        flags = []
        for name in ("depthTestFailed", "stencilTestFailed", "scissorClipped", "shaderDiscarded", "backfaceCulled"):
            if getattr(m, name, False):
                flags.append(name)
        w("event", m.eventId, "pre", pre, "post", post, "depth", round(m.postMod.depth, 6), " ".join(flags))
    ctl.Shutdown()
    cap.Shutdown()
except Exception:
    w(traceback.format_exc())
out.close()
os._exit(0)
