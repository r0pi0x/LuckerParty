import renderdoc as rd, os, traceback
out = open(os.environ["RD_OUT"], "w")
def w(*a):
    out.write(" ".join(str(x) for x in a) + "\n"); out.flush()
try:
    cap = rd.OpenCaptureFile()
    if cap.OpenFile(os.environ["RD_CAPTURE"], '', None) != rd.ResultCode.Succeeded:
        raise RuntimeError("open failed")
    result, ctl = cap.OpenCapture(rd.ReplayOptions(), None)
    if result != rd.ResultCode.Succeeded:
        raise RuntimeError(f"replay failed {result}")
    acts = []
    def walk(a):
        for x in a:
            acts.append(x); walk(x.children)
    walk(ctl.GetRootActions())
    draws = [a for a in acts if a.flags & rd.ActionFlags.Drawcall]
    w("actions", len(acts), "draws", len(draws))
    # The swapchain image: output of the last draw that has one.
    last = [a for a in draws if a.outputs[0] != rd.ResourceId.Null()][-1]
    target = last.outputs[0]
    texs = {t.resourceId: t for t in ctl.GetTextures()}
    t = texs[target]
    w("last draw", last.eventId, "target", t.width, "x", t.height, t.format.Name())
    px, py = int(os.environ.get("RD_X", t.width // 2)), int(os.environ.get("RD_Y", t.height // 2))
    # Follow full-screen copies back to the draw that shaded the pixel.
    eid = None
    for hop in range(6):
        hist = ctl.PixelHistory(target, px, py, rd.Subresource(0, 0, 0), rd.CompType.Typeless)
        passed = [h for h in hist if h.Passed()]
        w("hop", hop, "target", target, "history", [(h.eventId, h.Passed()) for h in hist][:12])
        if not passed:
            break
        cand = passed[-1]
        a = next(x for x in acts if x.eventId == cand.eventId)
        if a.numIndices == 3 and len(passed) == 1:
            ctl.SetFrameEvent(cand.eventId, True)
            st = ctl.GetPipelineState()
            srcs = [r.descriptor.resource for r in st.GetReadOnlyResources(rd.ShaderStage.Pixel)
                    if r.descriptor.resource != rd.ResourceId.Null()]
            if srcs:
                target = srcs[0]
                continue
        # The first passing draw is the opaque surface under the pixel.
        world = [h.eventId for h in passed if h.eventId < 1100]
        eid = int(os.environ.get("RD_EID", world[-1]))
        w("surface draws at pixel:", [(h.eventId) for h in passed])
        break
    ctl.SetFrameEvent(eid, True)
    state = ctl.GetPipelineState()
    act = next(a for a in acts if a.eventId == eid)
    w("\n=== draw", eid, act.GetName(ctl.GetStructuredFile()), "indices", act.numIndices)
    for stage in [rd.ShaderStage.Vertex, rd.ShaderStage.Pixel]:
        refl = state.GetShaderReflection(stage)
        if refl is None:
            continue
        w("\n--- stage", stage, "entry", refl.entryPoint)
        for ro in state.GetReadOnlyResources(stage):
            for desc in ro.access and [ro] or []:
                pass
        try:
            for i, res in enumerate(state.GetReadOnlyResources(stage)):
                d = res.descriptor
                if d.resource != rd.ResourceId.Null() and d.resource in texs:
                    tt = texs[d.resource]
                    w("  texture", i, ctl.GetResourceName(d.resource) if hasattr(ctl, "GetResourceName") else d.resource, tt.width, "x", tt.height, tt.format.Name())
        except Exception as e:
            w("  textures:", e)
        pipe = state.GetGraphicsPipelineObject()
        for ci, cb in enumerate(refl.constantBlocks):
            try:
                desc = state.GetConstantBlock(stage, ci, 0).descriptor
                vars_ = ctl.GetCBufferVariableContents(pipe, refl.resourceId, stage, refl.entryPoint, ci,
                                                       desc.resource, desc.byteOffset, desc.byteSize)
                def dump(vs, indent="  "):
                    for v in vs:
                        if v.members:
                            w(indent + v.name + ":"); dump(v.members, indent + "  ")
                        else:
                            vals = [v.value.f32v[k] for k in range(v.rows * v.columns)]
                            w(indent + v.name, [round(x, 6) for x in vals])
                w("  cbuffer", ci, cb.name)
                dump(vars_)
            except Exception as e:
                w("  cbuffer", ci, "error", e)
        targets = ctl.GetDisassemblyTargets(True)
        dis = ctl.DisassembleShader(pipe, refl, targets[0])
        open(os.environ["RD_OUT"] + f".{str(stage).split('.')[-1]}.txt", "w").write(dis)
        w("  disassembly ->", targets[0], len(dis), "chars")
    ctl.Shutdown(); cap.Shutdown()
except Exception:
    w(traceback.format_exc())
out.close()
os._exit(0)
