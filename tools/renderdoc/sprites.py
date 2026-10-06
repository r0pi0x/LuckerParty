"""List the draws that sample a texture of a given size (default: 128x128,
the size of materials/sprites/glow.vtf), with their blend and depth state
and the vertex shader's outputs for the first vertices (position and the
vertex colour the sprite carries). RD_W, RD_H pick the texture size.
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
    want = (int(os.environ.get("RD_W", "128")), int(os.environ.get("RD_H", "128")))
    cap = rd.OpenCaptureFile()
    cap.OpenFile(os.environ["RD_CAPTURE"], "", None)
    _, ctl = cap.OpenCapture(rd.ReplayOptions(), None)
    textures = {t.resourceId: t for t in ctl.GetTextures()}
    for a in actions(ctl.GetRootActions()):
        if not (a.flags & rd.ActionFlags.Drawcall):
            continue
        ctl.SetFrameEvent(a.eventId, True)
        st = ctl.GetPipelineState()
        reads = [r.descriptor.resource for r in st.GetReadOnlyResources(rd.ShaderStage.Pixel)]
        hit = [textures[r] for r in reads if r in textures and (textures[r].width, textures[r].height) == want]
        if not hit:
            continue
        w("draw", a.eventId, a.GetName(ctl.GetStructuredFile()), "verts", a.numIndices, "instances", a.numInstances)
        for t in hit:
            w("  texture", t.resourceId, t.width, "x", t.height, t.format.Name())
        bs = st.GetColorBlends()
        if bs:
            b = bs[0]
            w("  blend", b.enabled, b.colorBlend.source, b.colorBlend.destination, b.colorBlend.operation,
              "alpha", b.alphaBlend.source, b.alphaBlend.destination)
        ds = st.GetDepthState() if hasattr(st, "GetDepthState") else None
        if ds is not None:
            w("  depth test", ds.depthEnable, ds.depthFunction, "write", ds.depthWrites)
        # Vertex shader outputs (post-transform) for the first vertices.
        try:
            mesh = ctl.GetPostVSData(0, 0, rd.MeshDataStage.VSOut)
            if mesh.vertexResourceId != rd.ResourceId.Null():
                data = ctl.GetBufferData(mesh.vertexResourceId, mesh.vertexByteOffset, mesh.vertexByteStride * 4)
                import struct
                floats = mesh.vertexByteStride // 4
                for v in range(min(4, a.numIndices)):
                    vals = struct.unpack_from("<%df" % floats, data, v * mesh.vertexByteStride)
                    w("  vsout", v, ["%.4g" % x for x in vals])
        except Exception as e:
            w("  vsout error", e)
        # Pixel shader constants.
        try:
            refl = st.GetShaderReflection(rd.ShaderStage.Pixel)
            for i, cb in enumerate(refl.constantBlocks if refl else []):
                bind = st.GetConstantBlock(rd.ShaderStage.Pixel, i, 0)
                vars_ = ctl.GetCBufferVariableContents(
                    st.GetGraphicsPipelineObject(), refl.resourceId, rd.ShaderStage.Pixel,
                    st.GetShaderEntryPoint(rd.ShaderStage.Pixel), i,
                    bind.descriptor.resource, bind.descriptor.byteOffset, bind.descriptor.byteSize)
                for v in vars_:
                    vals = [v.value.f32v[k] for k in range(v.rows * v.columns)] if v.rows else []
                    w("  ps const", v.name, ["%.4g" % x for x in vals[:8]])
        except Exception as e:
            w("  ps const error", e)
    ctl.Shutdown()
    cap.Shutdown()
except Exception:
    w(traceback.format_exc())
out.close()
os._exit(0)
