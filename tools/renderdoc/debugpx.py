import renderdoc as rd, os, traceback
out = open(os.environ["RD_OUT"], "w")
def w(*a): out.write(" ".join(str(x) for x in a) + "\n"); out.flush()
try:
    cap = rd.OpenCaptureFile(); cap.OpenFile(os.environ["RD_CAPTURE"], '', None)
    _, ctl = cap.OpenCapture(rd.ReplayOptions(), None)
    ctl.SetFrameEvent(int(os.environ["RD_EID"]), True)
    x, y = int(os.environ["RD_X"]), int(os.environ["RD_Y"])
    inputs = rd.DebugPixelInputs()
    trace = ctl.DebugPixel(x, y, inputs)
    if trace is None or trace.debugger is None:
        w("no trace"); raise SystemExit
    w("inputs:")
    for v in trace.inputs:
        w("  ", v.name, [round(v.value.f32v[k], 5) for k in range(v.columns)])
    steps = 0
    while True:
        states = ctl.ContinueDebug(trace.debugger)
        if not states:
            break
        for s in states:
            steps += 1
            for c in s.changes:
                a = c.after
                if a.type == rd.VarType.Float and a.columns >= 3 and not a.members:
                    vals = [round(a.value.f32v[k], 5) for k in range(a.columns)]
                    w(f"  step {s.stepIndex} inst {s.nextInstruction}", a.name, vals)
    w("steps", steps)
    ctl.FreeTrace(trace)
    ctl.Shutdown(); cap.Shutdown()
except SystemExit:
    pass
except Exception:
    w(traceback.format_exc())
out.close(); os._exit(0)
