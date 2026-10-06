"""M13 pass/fail map: shots from one spot at every aim whose first layer has
the given surface props, through weapmeas.wall_pass; JSON lines out.
python3 tools/css_probe/passmap.py x y eye_z yaw0 dyaw props tmin tmax weapon... > out.jsonl
(NOTOOLS=1 skips aims whose next surface is a tools texture, which may give
no impact.) Summarise with passmap_summary.py out.jsonl."""
import sys, json
sys.path.insert(0, __import__('os').path.dirname(__file__))
import weapmeas as m
# usage: passmap.py x y eyez yaw0 dyaw props(comma) tmin tmax weapons...
x, y, ez, yaw0, dy = map(float, sys.argv[1:6])
props = tuple(int(p) for p in sys.argv[6].split(','))
tmin, tmax = float(sys.argv[7]), float(sys.argv[8])
import os
NOTOOLS = os.environ.get("NOTOOLS") == "1"
weapons = sys.argv[9:]
e = (x, y, ez)
g = m.trace((x, y, ez), 90, 0)['end'][2]
foot = (x, y, g + 0.1)
c = m.aim_catalog(e, [yaw0 + d for d in range(-int(dy), int(dy) + 1, 3)], list(range(-20, 41, 3)), mat_props=props,
                  tmin=tmin, tmax=tmax)
if NOTOOLS:
    c = [a for a in c if not a[4][4].startswith("TOOLS")]
print("catalog", len(c), sorted(round(a[0], 1) for a in c), flush=True)
aims = [(a[1], a[2]) for a in c]
for w in weapons:
    chunk = 40 if w in m.ZOOMED else 120
    for i in range(0, len(aims), chunk):
        for r in m.wall_pass(w, foot, aims[i:i + chunk]):
            l = r[4]
            print(json.dumps(dict(w=w, p=r[2], y=r[3], layers=l[:2], imps=r[5][:4], passed=r[6])), flush=True)
