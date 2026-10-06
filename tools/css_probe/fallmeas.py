"""Fall damage on the CS:S probe server (specs/cs_source/fall_damage.md).

Places the probe bot just above flat floor with a chosen downward speed, so
it lands on the first tick with that fall speed, and reads the damage from
the run's player_hurt event (mashup_wrun logging). The bot's health is set
to 1000 first (`mashup_health`, which also respawns it), so
damage past 100 is measured too and the bot survives.

  python3 tools/css_probe/fallmeas.py [speed ...]

Without speeds it sweeps 500..1200 in steps of 10. Prints
"speed landing_vz damage health" per run. Nothing is written into the repo.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
from weapcmp import SERVER, parse, rcon  # noqa: E402

import time  # noqa: E402

# Flat tile floor inside the CT spawn building on de_dust2 (z -126.24; the
# ceiling is at 64, irrelevant with a start speed).
SPOT = (352.0, 2384.0, -126.24)


def drop(speed, height=40.0, ticks=12, health=1000, armor=0):
    d = os.path.join(SERVER, "mashup")
    os.makedirs(d, exist_ok=True)
    rcon(f"mashup_health {health} {armor}")
    time.sleep(0.2)
    x, y, z = SPOT
    lines = [f"start {x} {y} {z + height} 0 0 0 0 {-speed}"]
    lines += ["0 0 0 0 0"] * ticks
    name = "fall"
    with open(os.path.join(d, name + ".in"), "w") as fh:
        fh.write("\n".join(lines) + "\n")
    out = os.path.join(d, name + ".out")
    if os.path.exists(out):
        os.remove(out)
    rcon(f"mashup_wrun mashup/{name}.in mashup/{name}.out")
    t0 = time.time()
    while True:
        time.sleep(0.2)
        try:
            r = parse(open(out).read())
        except FileNotFoundError:
            continue
        if ticks in r.w:
            break
        if time.time() - t0 > 20:
            raise RuntimeError("run timed out")
    hurt = [e for e in r.ev("player_hurt")]
    dmg = sum(int(e[2][e[2].index("dmg_health") + 1]) for e in hurt)
    health = int(hurt[-1][2][hurt[-1][2].index("health") + 1]) if hurt else None
    # The tick it landed: first grounded row; fall speed = -vz the row before.
    land = min((t for t, v in r.ticks.items() if v[7] == 1), default=None)
    before = r.ticks.get(land - 1) if land is not None else None
    return dmg, health, land, before[6] if before else None, [e[0] for e in hurt]


if __name__ == "__main__":
    if sys.argv[1:2] == ["armor"]:
        # Armour doesn't absorb fall damage; 100 health dies at the fatal speed.
        for armor, health, speed in [(0, 1000, 800), (100, 1000, 800), (0, 100, 940), (0, 100, 960), (0, 100, 990)]:
            dmg, h, land, vz, _ = drop(speed, health=health, armor=armor)
            print(f"armor {armor} health {health} fall {-vz} damage {dmg} health after {h}")
            print(rcon("mashup_info").strip() or "bot dead")
        sys.exit()
    speeds = [float(s) for s in sys.argv[1:]] or [float(s) for s in range(500, 1210, 10)]
    for s in speeds:
        dmg, health, land, vz, ticks = drop(s)
        print(f"speed {s:.1f} landed tick {land} prev_vz {vz} damage {dmg} health {health} hurt_ticks {ticks}")
