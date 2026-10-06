"""Weapon measurements on the CS:S probe server (specs/cs_source/weapons.md,
"Measurement plan" and "CS:S values (measured)").

Drives tools/css_probe/mashup_probe.sp over RCON: the first living bot is
the shooter, the next living player the target. Each scenario is one
mashup_run with weapon logging on; this module parses its rows:

  tick row:  "<tick> x y z vx vy vz onground ducked hull_top eye buttons water ladder"
  W row:     "W <tick> <weapon> clip .. reserve .. tickbase .. next_primary .. ..."
  E row:     "E <tick> <event> <client> ..."

Run measurements: python3 tools/css_probe/weapcmp.py <function> [...] with a
function name from weapmeas.py (e.g. knife, knife_range, damage). Results
print as text; nothing is written into the repo. Needs the probe server
running (tools/css_probe/README.md); a second bot on the other team is added
when missing (`bot_join_team any`, `bot_add_t`, mp_friendlyfire 1).
"""

import math
import os
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
from rcon import Rcon  # noqa: E402

SERVER = os.path.expanduser("~/games/css-ds/cstrike")
TICK = 0.015
IN_ATTACK, IN_JUMP, IN_DUCK, IN_ATTACK2, IN_RELOAD = 1, 2, 4, 2048, 8192

_rcon = None


def rcon(cmd):
    global _rcon
    for _ in range(3):
        try:
            if _rcon is None:
                _rcon = Rcon()
            return _rcon.exec(cmd)
        except Exception:
            _rcon = None
            time.sleep(1)
    raise RuntimeError("rcon failed: " + cmd)


class Run:
    def __init__(self):
        self.ticks = {}
        self.w = {}
        self.events = []

    def ev(self, name):
        return [e for e in self.events if e[1] == name]


def parse(text):
    r = Run()
    lines = text.split("\n")
    for line in lines[:-1]:  # the last piece may be a line still being written
        p = line.split()
        if not p:
            continue
        if p[0] == "W":
            d = {"tick": int(p[1]), "weapon": p[2]}
            i = 3
            while i < len(p):
                k = p[i]
                vals = []
                i += 1
                while i < len(p) and not p[i][0].isalpha():
                    vals.append(float(p[i]))
                    i += 1
                d[k] = vals[0] if len(vals) == 1 else vals
            r.w[d["tick"]] = d
        elif p[0] == "E":
            r.events.append((int(p[1]), p[2], p[3:]))
        else:
            v = [float(x) for x in p]
            r.ticks[int(v[0])] = v
    return r


def run(name, inputs, start=None, timeout=60):
    """inputs: list of (buttons, forward, side, pitch, yaw[, 1 + weapon slot to
    select]). start: (x, y, z,
    pitch, yaw) or None to leave the shooter where it is."""
    d = os.path.join(SERVER, "mashup")
    os.makedirs(d, exist_ok=True)
    lines = []
    if start is not None:
        x, y, z, pitch, yaw = start
        lines.append(f"start {x} {y} {z} {pitch} {yaw} 0 0 0")
    for row in inputs:
        b, f, s, pitch, yaw = row[:5]
        sel = f" {row[5]}" if len(row) > 5 else ""
        lines.append(f"{b} {f} {s} {pitch:.6f} {yaw:.6f}{sel}")
    with open(os.path.join(d, name + ".in"), "w") as fh:
        fh.write("\n".join(lines) + "\n")
    out = os.path.join(d, name + ".out")
    if os.path.exists(out):
        os.remove(out)
    rcon(f"mashup_wrun mashup/{name}.in mashup/{name}.out")
    want = len(inputs)
    t0 = time.time()
    while True:
        time.sleep(0.2)
        try:
            text = open(out).read()
        except FileNotFoundError:
            text = ""
        r = parse(text)
        if want in r.w:
            # Let the "done" close flush.
            return r
        if time.time() - t0 > timeout:
            raise RuntimeError(f"{name}: logged {len(r.w)} of {want} ticks")


def idle(n, pitch, yaw, buttons=0):
    return [(buttons, 0, 0, pitch, yaw)] * n


def trace(pos, pitch, yaw, ignore_shooter=True):
    out = rcon(f"mashup_trace {pos[0]} {pos[1]} {pos[2]} {pitch} {yaw} {1 if ignore_shooter else 0}")
    for line in out.splitlines():
        if line.startswith("mashup_trace"):
            p = line.split()
            return {
                "end": tuple(float(x) for x in p[1:4]),
                "ent": int(p[5]),
                "group": int(p[7]),
                "frac": float(p[9]),
                "startsolid": p[11] == "1",
                "surf": p[-1],
            }
    raise RuntimeError(out)


def angles_to(src, dst):
    dx, dy, dz = (dst[i] - src[i] for i in range(3))
    yaw = math.degrees(math.atan2(dy, dx))
    pitch = -math.degrees(math.atan2(dz, math.hypot(dx, dy)))
    return pitch, yaw


def dist(a, b):
    return math.sqrt(sum((a[i] - b[i]) ** 2 for i in range(3)))


def give(weapon, clip=None, reserve=None):
    cmd = f"mashup_give {weapon}"
    if clip is not None:
        cmd += f" {clip}"
        if reserve is not None:
            cmd += f" {reserve}"
    return rcon(cmd)


def target(pos, yaw=0.0, health=1000, armor=0, helmet=0, hover=1):
    return rcon(f"mashup_target {pos[0]} {pos[1]} {pos[2]} {yaw} {health} {armor} {helmet} {hover}")


def ensure_bots():
    st = rcon("status")
    if st.count("BOT") < 2:
        rcon("mp_friendlyfire 1")
        rcon("bot_join_team any")
        rcon("bot_add_t")
        time.sleep(2)


if __name__ == "__main__":
    import weapmeas

    ensure_bots()
    for name in sys.argv[1:]:
        getattr(weapmeas, name)()
