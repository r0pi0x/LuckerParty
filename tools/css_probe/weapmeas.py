"""Measurements for specs/cs_source/weapons.md ("CS:S values (measured)").
Each function prints its raw results; run through weapcmp.py."""

import math

from weapcmp import (IN_ATTACK, IN_ATTACK2, IN_DUCK, IN_RELOAD, angles_to, dist, give, idle, rcon, run,
                     target, trace)

# Flat, open spot: dust2 CT spawn, looking along yaw 200.
SPAWN = (352.18, 2462.96, -116.45)
EYE_H = 64.0


def fwd(p, yaw, d, dz=0.0):
    return (p[0] + d * math.cos(math.radians(yaw)), p[1] + d * math.sin(math.radians(yaw)), p[2] + dz)


def hurts(r, tick=None):
    out = []
    for t, _, a in r.ev("player_hurt"):
        if tick is not None and t != tick:
            continue
        d = dict(zip(a[1::2], a[2::2]))
        out.append((t, int(d["dmg_health"]), int(d["dmg_armor"]), int(d["hitgroup"])))
    return out


def knife_hit(button, d, back, aim_z=48.0, armor=0, yaw=200.0, settle=80, after=120):
    """One knife attack at a target whose origin is d units ahead."""
    give("weapon_knife")
    tpos = fwd(SPAWN, yaw, d)
    target(tpos, yaw=(yaw if back else yaw + 180.0), armor=armor, helmet=1 if armor else 0, hover=0)
    eye = (SPAWN[0], SPAWN[1], SPAWN[2] + EYE_H)
    pitch, _ = angles_to(eye, (tpos[0], tpos[1], tpos[2] + aim_z))
    ins = idle(settle, pitch, yaw) + [(button, 0, 0, pitch, yaw)] + idle(after, pitch, yaw)
    r = run("knife", ins, start=(SPAWN[0], SPAWN[1], SPAWN[2], pitch, yaw))
    return r, settle + 1


def knife():
    # Damage front/back, primary/secondary, armour.
    for button, name in ((IN_ATTACK, "slash"), (IN_ATTACK2, "stab")):
        for back in (False, True):
            for armor in (0, 100):
                r, t = knife_hit(button, 40.0, back, armor=armor)
                w = r.w[t]
                base = w["tickbase"] - 1
                np_ = round(w["next_primary"] - base * 0.015, 4)
                ns_ = round(w["next_secondary"] - base * 0.015, 4)
                print(name, "back" if back else "front", "armor", armor, hurts(r), "next_primary s", np_,
                      "next_secondary s", ns_)
    # Miss timings.
    for button, name in ((IN_ATTACK, "slash"), (IN_ATTACK2, "stab")):
        r, t = knife_hit(button, 200.0, False)
        w = r.w[t]
        base = w["tickbase"] - 1
        print(name, "miss", hurts(r), "next_primary s", round(w["next_primary"] - base * 0.015, 4),
              "next_secondary s", round(w["next_secondary"] - base * 0.015, 4))


def knife_range(aims=(48.0, 64.0), lo=40, hi=100):
    """Largest target distance that still hits, by bisection, and the
    straight-ray distance from the eye to the hitbox at that point."""
    eye = (SPAWN[0], SPAWN[1], SPAWN[2] + EYE_H)
    for button, name in ((IN_ATTACK, "slash"), (IN_ATTACK2, "stab")):
        for aim_z in aims:
            a, b = float(lo), float(hi)
            while b - a > 0.25:
                m = (a + b) / 2
                r, t = knife_hit(button, m, False, aim_z=aim_z, after=5)
                if hurts(r):
                    a = m
                else:
                    b = m
            tpos = fwd(SPAWN, 200.0, a)
            target(tpos, yaw=20.0, hover=0)
            pitch, yaw = angles_to(eye, (tpos[0], tpos[1], tpos[2] + aim_z))
            tr = trace(eye, pitch, yaw)
            print(name, "aim_z", aim_z, "max hit distance (origins)", a, "pitch", round(pitch, 2), "ray to hitbox",
                  round(dist(eye, tr["end"]), 2), "group", tr["group"])


def knife_repeat():
    for button, name in ((IN_ATTACK, "slash"), (IN_ATTACK2, "stab")):
        for d in (40.0, 200.0):
            give("weapon_knife")
            yaw = 200.0
            tpos = fwd(SPAWN, yaw, d)
            target(tpos, yaw=yaw + 180.0, hover=0)
            ins = idle(80, 10, yaw) + idle(300, 10, yaw, buttons=button)
            r = run("knife_rep", ins, start=(SPAWN[0], SPAWN[1], SPAWN[2], 0, yaw))
            # Swings: next_primary changes upward.
            swings = []
            prev = None
            for t in sorted(r.w):
                v = (r.w[t]["next_primary"], r.w[t]["next_secondary"])
                if prev is not None and v != prev and t > 70:
                    swings.append(t)
                prev = v
            print(name, "d", d, "swing ticks", swings[:12], "gaps",
                  [b - a for a, b in zip(swings, swings[1:])][:11], "damage", [x[1] for x in hurts(r)][:12])


GAPS = (34, 40, 60, 100)


def knife_pause():
    """Second slash damage vs. pause after the first."""
    yaw = 200.0
    for gap in GAPS:
        give("weapon_knife")
        target(fwd(SPAWN, yaw, 40.0), yaw=yaw + 180.0, hover=0)
        ins = idle(80, 10, yaw) + [(IN_ATTACK, 0, 0, 10, yaw)] + idle(gap - 1, 10, yaw) + \
            [(IN_ATTACK, 0, 0, 10, yaw)] + idle(40, 10, yaw) + [(IN_ATTACK, 0, 0, 10, yaw)] + idle(40, 10, yaw)
        r = run("knife_pause", ins, start=(SPAWN[0], SPAWN[1], SPAWN[2], 10, yaw))
        print("gap", gap, [(t, h) for t, h, _, _ in hurts(r)])


def knife_backstab_angle(deltas=range(0, 181, 10), button=IN_ATTACK2, d=40.0, side=0.0, aim_center=True):
    """Stab damage vs. target yaw relative to the attacker's (0 = facing away).
    side: lateral offset of the target (to test position vs. facing)."""
    yaw = 200.0
    for delta in deltas:
        give("weapon_knife")
        p = fwd(SPAWN, yaw, d)
        p = fwd(p, yaw + 90.0, side)
        target(p, yaw=yaw + delta, hover=0)
        eye = (SPAWN[0], SPAWN[1], SPAWN[2] + EYE_H)
        pitch, aim = angles_to(eye, (p[0], p[1], p[2] + 40))
        if not aim_center:
            aim = yaw
        ins = idle(80, pitch, aim) + [(button, 0, 0, pitch, aim)] + idle(10, pitch, aim)
        r = run("knife_bs", ins, start=(SPAWN[0], SPAWN[1], SPAWN[2], pitch, aim))
        print("delta", delta, "side", side, [h for _, h, _, _ in hurts(r)], [e[2][-5:-2] for e in r.ev("player_hurt")])


# Long open line (on a clip brush above dust2's middle; bullets fly 4269
# units along yaw 195 before reaching the skybox).
RANGE_POS = (956.0, 88.0, 385.0)
RANGE_YAW = 195.0
GROUPS = {1: "head", 2: "chest", 3: "stomach", 4: "left arm", 5: "right arm", 6: "left leg", 7: "right leg"}


def shooter_eye(duck):
    r = run("eye", idle(30, 0, RANGE_YAW, buttons=IN_DUCK if duck else 0),
            start=(RANGE_POS[0], RANGE_POS[1], RANGE_POS[2], 0, RANGE_YAW))
    return tuple(r.w[30]["eye"])


def aim_map(eye, tpos, n=24):
    """Aim angles that hit each hitgroup, the most interior one per group."""
    pitch_c, yaw_c = angles_to(eye, (tpos[0], tpos[1], tpos[2] + 36))
    d = dist(eye, tpos)
    half_w = math.degrees(math.atan2(24, d))
    half_h = math.degrees(math.atan2(44, d))
    cells = {}
    for i in range(n + 1):
        for j in range(2 * n + 1):
            yaw = yaw_c - half_w + 2 * half_w * i / n
            pitch = pitch_c - half_h + 2 * half_h * j / (2 * n)
            t = trace(eye, pitch, yaw)
            cells[(i, j)] = (t["group"] if t["ent"] > 0 else -1, pitch, yaw)
    best = {}
    for (i, j), (g, pitch, yaw) in cells.items():
        if g <= 0:
            continue
        # Distance (in cells) to the nearest cell of another group.
        m = min(((i - a) ** 2 + ((j - b) / 2) ** 2 for (a, b), (g2, _, _) in cells.items() if g2 != g),
                default=0)
        if g not in best or m > best[g][0]:
            best[g] = (m, pitch, yaw)
    return {g: (p, y) for g, (_, p, y) in best.items()}


def damage(weapon="weapon_ak47", dists=(100, 500, 1000, 2000, 3000), armor=0, helmet=0, shots=6, gap=40,
           duck=True, groups=(1, 2, 3, 4, 5, 6, 7)):
    """Shots at each hitgroup of a target at several distances. Prints one row
    per hit: distance (eye to impact), hitgroup, health and armour damage."""
    eye = shooter_eye(duck)
    rows = []
    for d in dists:
        tpos = (RANGE_POS[0] + d * math.cos(math.radians(RANGE_YAW)),
                RANGE_POS[1] + d * math.sin(math.radians(RANGE_YAW)), RANGE_POS[2])
        target(tpos, yaw=RANGE_YAW + 180.0, armor=armor, helmet=helmet, hover=1)
        aims = aim_map(eye, tpos)
        give(weapon, 200, 0)
        b = IN_DUCK if duck else 0
        ins = idle(90, 0, RANGE_YAW, buttons=b)
        for g in groups:
            if g not in aims:
                continue
            p, y = aims[g]
            for _ in range(shots):
                ins += idle(gap - 1, p, y, buttons=b) + [(b | IN_ATTACK, 0, 0, p, y)]
        ins += idle(10, 0, RANGE_YAW, buttons=b)
        r = run("dmg", ins, start=(RANGE_POS[0], RANGE_POS[1], RANGE_POS[2], 0, RANGE_YAW))
        impacts = {}
        for t, _, a in r.ev("bullet_impact"):
            impacts.setdefault(t, []).append(tuple(float(x) for x in a[1:4]))
        for t, h, ar, g in hurts(r):
            # Impact on the target: the one nearest the target's axis.
            imp = min(impacts.get(t, [tpos]), key=lambda q: math.hypot(q[0] - tpos[0], q[1] - tpos[1]))
            sh_eye = r.w[t]["eye"]
            rows.append((weapon, d, round(dist(sh_eye, imp), 1), g, h, ar, armor, helmet))
            print(weapon, "d", d, "dist", round(dist(sh_eye, imp), 1), "group", g, GROUPS.get(g), "health", h,
                  "armor", ar, "(armor", armor, "helmet", helmet, ")")
    return rows


SLOT = {"weapon_knife": 2}
PISTOLS = ("weapon_usp", "weapon_glock", "weapon_deagle", "weapon_p228", "weapon_elite", "weapon_fiveseven")


def fire_ticks(r):
    return [t for t, _, _ in r.ev("weapon_fire")]


def timing(weapon, clip=10, reserve=90, semi=None, hold=260):
    """Deploy, refire, empty-clip and reload timing, all in ticks."""
    slot = 1 if weapon in PISTOLS else 0
    if semi is None:
        semi = weapon in PISTOLS or weapon in ("weapon_awp", "weapon_scout")
    give(weapon, clip, reserve)
    p, y = -10.0, RANGE_YAW
    ins = idle(80, p, y)
    ins.append((0, 0, 0, p, y, 3))            # tick 81: knife
    ins += idle(79, p, y)
    sel = len(ins) + 1
    # tick sel: select the gun and keep attacking
    for k in range(hold):
        b = IN_ATTACK if (not semi or k % 2 == 0) else 0
        row = (b, 0, 0, p, y)
        if k == 0:
            row = row + (slot + 1,)
        ins.append(row)
    ins += idle(20, p, y)
    rl = len(ins) + 1
    ins.append((IN_RELOAD, 0, 0, p, y))
    for k in range(400):
        b = IN_ATTACK if (not semi or k % 2 == 0) else 0
        ins.append((b, 0, 0, p, y))
    r = run("timing", ins, start=(RANGE_POS[0], RANGE_POS[1], RANGE_POS[2], p, y))
    fires = fire_ticks(r)
    first = [t for t in fires if t >= sel]
    w = r.w
    print(weapon, "select at", sel, "first shot", first[0] if first else None,
          "deploy ticks", (first[0] - sel) if first else None,
          "next_attack after select (s)", round(w[sel]["next_attack"] - (w[sel]["tickbase"] - 1) * 0.015, 4))
    pre = [t for t in first if t < rl]
    print("  shots before reload", len(pre), "intervals", [b - a for a, b in zip(pre, pre[1:])])
    clip_after = [(t, w[t]["clip"], w[t]["reserve"]) for t in range(sel, len(ins) + 1)
                  if t - 1 in w and (w[t]["clip"], w[t]["reserve"]) != (w[t - 1]["clip"], w[t - 1]["reserve"])]
    print("  clip/reserve changes", clip_after[-6:])
    post = [t for t in first if t >= rl]
    print("  reload pressed", rl, "first shot after", post[0] if post else None,
          "ticks", (post[0] - rl) if post else None,
          "next_attack after reload (s)", round(w[rl]["next_attack"] - (w[rl]["tickbase"] - 1) * 0.015, 4),
          "intervals after", [b - a for a, b in zip(post, post[1:])][:8])
    reloads = [t for t, _, _ in r.ev("weapon_reload")]
    print("  weapon_reload events", reloads)
    return r


IN_SPEED = 1 << 17
IN_JUMP = 2


def penalty_trace(weapon="weapon_ak47"):
    """m_fAccuracyPenalty per tick through stand, crouch, walk, run, jump,
    land and firing. Prints compact runs of (tick, state, speed, penalty)."""
    give(weapon, 200, 0)
    y, p = 220.0, 0.0
    seq = []
    seq += [("stand", 0, 0)] * 100
    seq += [("crouch", IN_DUCK, 0)] * 100
    seq += [("stand", 0, 0)] * 100
    seq += [("walk", IN_SPEED, 1)] * 60 + [("walk", IN_SPEED, -1)] * 60
    seq += [("stop", 0, 0)] * 80
    seq += [("run", 0, 1)] * 60 + [("run", 0, -1)] * 60
    seq += [("stop", 0, 0)] * 80
    seq += [("jump", IN_JUMP, 0)] + [("air", 0, 0)] * 120
    seq += [("fire1", IN_ATTACK, 0)] + [("after1", 0, 0)] * 100
    seq += [("fire5", IN_ATTACK, 0)] * 30 + [("after5", 0, 0)] * 100
    seq += [("cfire1", IN_ATTACK | IN_DUCK, 0)] + [("cafter1", IN_DUCK, 0)] * 100
    ins = idle(80, p, y)
    for _, b, f in seq:
        ins.append((b, 250.0 * f, 0, p, y))
    r = run("pen", ins, start=(SPAWN[0], SPAWN[1], SPAWN[2], p, y))
    out = []
    for i, (state, _, _) in enumerate(seq):
        t = 81 + i
        v = r.ticks[t]
        out.append((t, state, round(math.hypot(v[4], v[5]), 2), int(v[7]), int(v[8]), r.w[t]["penalty"],
                    r.w[t]["shots"]))
    return out, r


def deploy(weapon, semi=False):
    """Draw time: the bot holds only its knife, the gun is given mid-run
    (the plugin draws it 0.1 s later) while attack is held/tapped."""
    import threading
    import time as _t
    give("weapon_knife")
    _t.sleep(0.5)
    p, y = -10.0, RANGE_YAW
    ins = []
    for k in range(400):
        b = IN_ATTACK2 * 0  # no knife swings before the gun arrives
        if k > 0:
            b = IN_ATTACK if (not semi or k % 2 == 0) else 0
        ins.append((b if k > 100 else 0, 0, 0, p, y))
    th = threading.Timer(1.0, lambda: give(weapon, 10, 90))
    th.start()
    r = run("deploy", ins, start=(RANGE_POS[0], RANGE_POS[1], RANGE_POS[2], p, y))
    th.join()
    sw = next(t for t in sorted(r.w) if r.w[t]["weapon"] == weapon)
    w = r.w[sw]
    fires = [t for t, _, a in r.ev("weapon_fire") if a[1] == weapon.replace("weapon_", "")]
    print(weapon, "drawn at tick", sw, "next_attack", round(w["next_attack"] - (w["tickbase"] - 1) * 0.015, 4),
          "next_primary", round(w["next_primary"] - (w["tickbase"] - 1) * 0.015, 4),
          "first shot tick", fires[0] if fires else None, "ticks after draw", (fires[0] - sw) if fires else None)
    return r


def refire(weapon, mode="hold", n=200, clip=30, period=2):
    """Fire ticks with attack held ("hold") or pressed every `period` ticks ("tap")."""
    give(weapon, clip, 0)
    p, y = -10.0, RANGE_YAW
    ins = idle(90, p, y)
    for k in range(n):
        if mode == "hold":
            b = IN_ATTACK
        else:
            b = IN_ATTACK if k % period == 0 else 0
        ins.append((b, 0, 0, p, y))
    r = run("refire", ins, start=(RANGE_POS[0], RANGE_POS[1], RANGE_POS[2], p, y))
    f = [t for t, _, a in r.ev("weapon_fire")]
    w = r.w
    nps = [round(w[t]["next_primary"] - (w[t]["tickbase"] - 1) * 0.015, 4) for t in f]
    print(weapon, mode, period, "shots", len(f), "intervals", [b - a for a, b in zip(f, f[1:])][:16], "next_primary after shot",
          nps[:6])
    return r


def offsets(r, fire_ticks):
    """Per shot: (tick, offset magnitude in tangent units, (right, up) offset,
    penalty before the shot, horizontal speed, punch before the shot)."""
    imps = {}
    for t, _, a in r.ev("bullet_impact"):
        imps.setdefault(t, tuple(float(x) for x in a[1:4]))
    out = []
    for t in fire_ticks:
        if t not in imps or t - 1 not in r.w:
            continue
        w0 = r.w[t - 1]
        w = r.w[t]
        eye = w["eye"]
        pitch, yaw = w["ang"]
        pp = w0["punch"]
        ap, ay = math.radians(pitch), math.radians(yaw)
        f = (math.cos(ap) * math.cos(ay), math.cos(ap) * math.sin(ay), -math.sin(ap))
        right = (math.sin(ay), -math.cos(ay), 0.0)
        up = (math.sin(ap) * math.cos(ay), math.sin(ap) * math.sin(ay), math.cos(ap))
        d = [imps[t][i] - eye[i] for i in range(3)]
        k = sum(d[i] * f[i] for i in range(3))
        v = [d[i] / k - f[i] for i in range(3)]
        x = sum(v[i] * right[i] for i in range(3))
        y = sum(v[i] * up[i] for i in range(3))
        sp = math.hypot(r.ticks[t][4], r.ticks[t][5])
        out.append((t, math.hypot(x, y), (x, y), w0["penalty"], sp, pp, r.ticks[t][7], r.ticks[t][8]))
    return out


def spread(weapon="weapon_ak47", state="stand", n=100, gap=40, yaw=270.0):
    """Single shots at a wall; state: stand, crouch, run (moving at full
    speed, alternating direction), walk, air (shot 15 ticks after a jump)."""
    give(weapon, 250, 0)
    ins = idle(90, 0, yaw)
    for k in range(n):
        for j in range(gap):
            b, fw = 0, 0.0
            if state == "crouch":
                b = IN_DUCK
            if state in ("run", "walk") or state.startswith("speed"):
                v = float(state[5:]) if state.startswith("speed") else 250.0
                fw = v if (k % 2 == 0) else -v
                if state == "walk":
                    b |= IN_SPEED
                # side-step along the wall so the distance stays the same
            if state == "air" and j == gap - 16:
                b |= IN_JUMP
            if j == gap - 1:
                b |= IN_ATTACK
            ins.append((b, 0.0, fw, 0.0, yaw))
    r = run("spread", ins, start=(SPAWN[0], SPAWN[1], SPAWN[2], 0, yaw), timeout=300)
    return offsets(r, fire_ticks(r)), r


def recoil(weapon="weapon_ak47", shots=30, yaw=270.0, after=120, duck=False):
    """Hold attack for `shots` rounds, then release; per-tick punch and the
    per-shot bullet offsets in degrees relative to the view."""
    give(weapon, shots, 0)
    b = IN_DUCK if duck else 0
    ins = idle(90, 0, yaw, buttons=b) + idle(shots * 8, 0, yaw, buttons=b | IN_ATTACK) + idle(after, 0, yaw, buttons=b)
    r = run("recoil", ins, start=(SPAWN[0], SPAWN[1], SPAWN[2], 0, yaw))
    return r


def decay_punch(p, dt=0.015):
    ln = math.hypot(p[0], p[1])
    if ln == 0:
        return p
    nl = max(ln - (10.0 + 0.5 * ln) * dt, 0.0)
    return [p[0] * nl / ln, p[1] * nl / ln, 0.0]


def recoil_analysis(r):
    f = fire_ticks(r)
    o = {x[0]: x for x in offsets(r, f)}
    rows = []
    # decay check on non-fire ticks
    err = 0.0
    for t in sorted(r.w):
        if t - 1 in r.w and t not in f:
            pred = decay_punch(r.w[t - 1]["punch"])
            err = max(err, abs(pred[0] - r.w[t]["punch"][0]), abs(pred[1] - r.w[t]["punch"][1]))
    for i, t in enumerate(f):
        before = decay_punch(r.w[t - 1]["punch"])
        after = r.w[t]["punch"]
        kick = (round(before[0] - after[0], 4), round(after[1] - before[1], 4))
        res = None
        if t in o:
            x = o[t]
            # bullet angles relative to view (degrees, up and left positive)
            up = math.degrees(math.atan(x[2][1]))
            left = -math.degrees(math.atan(x[2][0]))
            # residual after view + k*punch (punch pitch negative = up, yaw positive = left)
            res = {k: (round(up + k * before[0], 3), round(left - k * before[1], 3)) for k in (1, 2)}
            cone = math.degrees(x[3] + 0.0006)
            res["cone_deg"] = round(cone, 3)
        rows.append((i + 1, t, "kick up,left", kick, "after", [round(v, 3) for v in after[:2]], res))
    return err, rows


def zoom(weapon):
    give(weapon, 10, 30)
    p, y = 0.0, 270.0
    ins = idle(100, p, y)
    marks = {}
    for k in range(3):
        marks[len(ins) + 1] = "attack2"
        ins += [(IN_ATTACK2, 0, 0, p, y)] + idle(99, p, y)
    marks[len(ins) + 1] = "attack2"
    ins += [(IN_ATTACK2, 0, 0, p, y)] + idle(99, p, y)
    marks[len(ins) + 1] = "fire"
    ins += [(IN_ATTACK, 0, 0, p, y)] + idle(200, p, y)
    r = run("zoom", ins, start=(SPAWN[0], SPAWN[1], SPAWN[2], p, y))
    prev = None
    out = []
    for t in sorted(r.w):
        w = r.w[t]
        cur = (w["fov"], w["mode"])
        if cur != prev or t in marks:
            out.append((t, marks.get(t, ""), "fov", w["fov"], "mode", w["mode"],
                        "next_primary", round(w["next_primary"] - (w["tickbase"] - 1) * 0.015, 3),
                        "next_secondary", round(w["next_secondary"] - (w["tickbase"] - 1) * 0.015, 3),
                        "velmod", w["velmod"]))
        prev = cur
    print(weapon)
    for o in out:
        print("  ", o)
    return r


def firemode(weapon, taps=6, period=2, hold=False):
    give(weapon, 30, 60)
    p, y = 0.0, 270.0
    ins = idle(100, p, y) + [(IN_ATTACK2, 0, 0, p, y)] + idle(249, p, y)
    start = len(ins) + 1
    for k in range(120):
        b = IN_ATTACK if (hold or (k % period == 0 and k // period < taps)) else 0
        ins.append((b, 0, 0, p, y))
    ins += idle(60, p, y)
    r = run("mode", ins, start=(SPAWN[0], SPAWN[1], SPAWN[2], p, y))
    w = r.w[102]
    base = (w["tickbase"] - 1) * 0.015
    print(weapon, "after attack2: special", w["special"], "mode", w["mode"], "next_primary", round(w["next_primary"] - base, 4),
          "next_secondary", round(w["next_secondary"] - base, 4), "next_attack", round(w["next_attack"] - base, 4))
    f = [t for t in fire_ticks(r) if t >= start]
    imp = {}
    for t, _, a in r.ev("bullet_impact"):
        imp[t] = imp.get(t, 0) + 1
    print("  fire ticks (rel. to first press)", [t - start for t in f], "impacts per tick",
          [(t - start, imp[t]) for t in sorted(imp) if t >= start][:20])
    print("  clip", r.w[len(ins)]["clip"])
    return r


# ---------------------------------------------------------------------------
# M13 penetration

def wall(eye, pitch, yaw):
    """mashup_wall along a ray: entry/exit points, thickness, surface props."""
    out = rcon(f"mashup_wall {eye[0]} {eye[1]} {eye[2]} {pitch} {yaw}")
    for line in out.splitlines():
        if line.startswith("mashup_wall"):
            p = line.split()
            if p[1] == "none":
                return None
            g = lambda k, n=1: [float(x) for x in p[p.index(k) + 1:p.index(k) + 1 + n]]
            return {"entry": tuple(g("entry", 3)), "exit": tuple(g("exit", 3)), "thick": g("thick")[0],
                    "dist": g("dist")[0], "props": int(g("props")[0]), "exitprops": int(g("exitprops")[0]),
                    "behind": g("behind")[0], "nextprops": int(g("nextprops")[0]),
                    "ent": p[p.index("ent") + 2], "surf": p[p.index("surf") + 1]}
    raise RuntimeError(out)


_SURF = None


def mat(props):
    global _SURF
    if _SURF is None:
        import surfprops
        _SURF = surfprops.table()
    n, m, _ = _SURF[props]
    return f"{n}({m})"


def surfaces(eye, pitch, yaw, n=6, maxd=8192):
    """All solid layers along a ray: list of (entry dist, thickness, props, exitprops)."""
    out = []
    a = math.radians(yaw)
    pr = math.radians(pitch)
    d = (math.cos(pr) * math.cos(a), math.cos(pr) * math.sin(a), -math.sin(pr))
    pos = eye
    travelled = 0.0
    for _ in range(n):
        w = wall(pos, pitch, yaw)
        if w is None:
            break
        travelled += w["dist"]
        out.append((round(travelled, 2), round(w["thick"], 2), w["props"], w["exitprops"], w["surf"]))
        travelled += w["thick"]
        if travelled > maxd:
            break
        pos = tuple(w["exit"][i] + d[i] * 0.05 for i in range(3))
        travelled += 0.05
    return out


ZOOMED = ("weapon_awp", "weapon_scout", "weapon_sg550", "weapon_g3sg1")


def pen_shots(weapon, foot, aim_points, duck=True, gap=60, tyaw=None, settle=90):
    """Shooter standing (crouched) at `foot`; for each aim point (x, y, z) the
    target is placed so that its chest (origin + 48) sits on the point, then
    one shot is fired at it. Returns per shot the hurt events and impacts."""
    b = IN_DUCK if duck else 0
    res = []
    eye = None
    for k, pt in enumerate(aim_points):
        if eye is None:
            r0 = run("pen_eye", idle(30, 0, 0, buttons=b), start=(foot[0], foot[1], foot[2], 0, 0))
            eye = tuple(r0.w[30]["eye"])
        pitch, yaw = angles_to(eye, pt)
        target((pt[0], pt[1], pt[2] - 48.0), yaw=(yaw + 180.0) if tyaw is None else tyaw, hover=1)
        give(weapon, 5, 0)
        ins = idle(settle, pitch, yaw, buttons=b)
        if weapon in ZOOMED:
            ins += [(b | IN_ATTACK2, 0, 0, pitch, yaw)] + idle(40, pitch, yaw, buttons=b)
        shot = len(ins) + 1
        ins += [(b | IN_ATTACK, 0, 0, pitch, yaw)] + idle(12, pitch, yaw, buttons=b)
        r = run("pen", ins, start=(foot[0], foot[1], foot[2], pitch, yaw))
        imps = [tuple(float(x) for x in a[1:4]) for t, _, a in r.ev("bullet_impact") if t == shot]
        h = hurts(r)
        res.append({"pt": pt, "eye": tuple(r.w[shot]["eye"]), "pitch": pitch, "yaw": yaw, "hurt": h,
                    "impacts": imps, "fired": bool([1 for t, _, _ in r.ev("weapon_fire") if t == shot])})
    return res


def pen_shot(weapon, foot, pt, chest=55.0, duck=True, settle=90, eye=None):
    """One shot from `foot` at a target whose back faces the shooter and
    whose origin + chest sits on `pt` (chest from behind, see M13)."""
    b = IN_DUCK if duck else 0
    if eye is None:
        r0 = run("pen_eye", idle(30, 0, 0, buttons=b), start=(foot[0], foot[1], foot[2], 0, 0))
        eye = tuple(r0.w[30]["eye"])
    pitch, yaw = angles_to(eye, pt)
    target((pt[0], pt[1], pt[2] - chest), yaw=yaw, hover=1)
    give(weapon, 5, 0)
    ins = idle(settle, pitch, yaw, buttons=b)
    if weapon in ZOOMED:
        ins += [(b | IN_ATTACK2, 0, 0, pitch, yaw)] + idle(40, pitch, yaw, buttons=b)
    shot = len(ins) + 1
    ins += [(b | IN_ATTACK, 0, 0, pitch, yaw)] + idle(12, pitch, yaw, buttons=b)
    r = run("pen", ins, start=(foot[0], foot[1], foot[2], pitch, yaw))
    imps = [tuple(float(x) for x in a[1:4]) for t, _, a in r.ev("bullet_impact") if t == shot]
    return {"eye": tuple(r.w[shot]["eye"]), "pitch": pitch, "yaw": yaw, "hurt": hurts(r), "impacts": imps,
            "fired": bool([1 for t, _, _ in r.ev("weapon_fire") if t == shot])}


def ground(x, y, ztop):
    t = trace((x, y, ztop), 90, 0)
    return t["end"][2]


WEAPON = {  # Damage, RangeModifier, Penetration (weapons.md tables)
    "weapon_ak47": (36, 0.98, 2), "weapon_awp": (115, 0.99, 3), "weapon_deagle": (54, 0.81, 2),
    "weapon_usp": (34, 0.79, 1), "weapon_m4a1": (33, 0.97, 2), "weapon_glock": (25, 0.75, 1),
}
GMUL = {0: 1, 1: 4, 2: 1, 3: 1.25, 4: 1, 5: 1, 6: 0.75, 7: 0.75}


def pen_series(weapon, hit, normal, thetas, d1, d2s, ztop=200.0, side=1, chest=55.0, tag="", quiet=False, level=True):
    """Shots through a surface at point `hit` (x, y, z where the line
    crosses the entry face) with horizontal entry normal `normal` (pointing to
    the shooter): shooter d1 away at angle theta from the normal, target d2
    behind the last layer the line crosses before d2. Prints one row per shot
    with every solid layer between the eye and the target. level: shoot
    horizontally (the crossing point takes the eye's height)."""
    nyaw = math.degrees(math.atan2(normal[1], normal[0]))
    rows = []
    for th in thetas:
        a = nyaw + side * th
        sx, sy = hit[0] + d1 * math.cos(math.radians(a)), hit[1] + d1 * math.sin(math.radians(a))
        tr = trace((sx, sy, ztop), 90, 0)
        if tr["startsolid"] or tr["frac"] >= 1.0:
            print("skip theta", th, "no ground")
            continue
        foot = (sx, sy, tr["end"][2] + 0.1)
        r0 = run("pen_eye", idle(30, 0, 0, buttons=IN_DUCK), start=(foot[0], foot[1], foot[2], 0, 0))
        eye = tuple(r0.w[30]["eye"])
        if math.hypot(eye[0] - sx, eye[1] - sy) > 1.0:
            print("skip theta", th, "shooter not placed")
            continue
        if level:
            hit = (hit[0], hit[1], eye[2])
        p0, y0 = angles_to(eye, hit)
        all_layers = surfaces(eye, p0, y0, n=5)
        dirv = [hit[i] - eye[i] for i in range(3)]
        ln = math.sqrt(sum(v * v for v in dirv))
        dirv = [v / ln for v in dirv]
        for d2 in d2s:
            layers = [l for l in all_layers if l[0] < ln + 60.0]
            if not layers:
                print("skip theta", th, "no layer")
                continue
            end = layers[-1][0] + layers[-1][1]
            nxt = [l for l in all_layers if l[0] > end]
            if nxt and nxt[0][0] < end + d2 + 20:
                print("skip theta", th, "d2", d2, "blocked at", nxt[0][0] - end)
                continue
            pt = tuple(eye[i] + dirv[i] * (end + d2) for i in range(3))
            s = pen_shot(weapon, foot, pt, chest=chest, eye=eye)
            dmg, rm, _ = WEAPON[weapon]
            lay = [(l[1], mat(l[2]), mat(l[3])) for l in layers]
            gap = [round(layers[i + 1][0] - layers[i][0] - layers[i][1], 1) for i in range(len(layers) - 1)]
            imp = s["impacts"]
            ti = min(imp, key=lambda q: dist(q, pt)) if imp else pt
            dt = dist(s["eye"], ti)
            row = dict(w=weapon, tag=tag, theta=th, d1=round(layers[0][0], 1), d2=d2, layers=lay, gaps=gap,
                       dist=round(dt, 1), nimp=len(imp))
            if s["hurt"]:
                (t, h, ar, g) = s["hurt"][0]
                base = dmg * rm ** (dt / 500.0) * GMUL[g]
                row.update(group=g, dmg=h, base=round(base, 2), ratio=round(h / base, 4))
            else:
                row.update(dmg=0, impacts=[tuple(round(c, 1) for c in q) for q in imp])
            rows.append(row)
            if not quiet:
                print(row)
    return rows


# ---------------------------------------------------------------------------
# M13 through players (collaterals)

def targetn(n, pos, yaw=0.0, hover=1):
    return rcon(f"mashup_targetn {n} {pos[0]} {pos[1]} {pos[2]} {yaw} {hover}")


def hurts_by_victim(r, tick):
    out = []
    for t, _, a in r.ev("player_hurt"):
        if t != tick:
            continue
        d = dict(zip(a[1::2], a[2::2]))
        out.append((int(a[0]), int(d["dmg_health"]), int(d["hitgroup"])))
    return out


def along(d, z):
    return (RANGE_POS[0] + d * math.cos(math.radians(RANGE_YAW)),
            RANGE_POS[1] + d * math.sin(math.radians(RANGE_YAW)), z)


def collateral(weapon, aim_z=55.0, first=300.0, gap=100.0, first_on=True, first_yaw=None, second_on=True,
               side=0.0, eye=None, shots=1):
    """Shooter crouched on the range line; target 1 (`first` units away) and
    target 2 (`gap` behind it) hover on the line, backs to the shooter, the
    line crossing both at height aim_z above their origins. A target that is
    off is parked 300 units to the side. side: lateral aim offset (units,
    at the targets). Returns (client of target 1, client of target 2, rows of
    (victim, dmg, group, eye->impact distance), impacts)."""
    if eye is None:
        eye = shooter_eye(True)
    z = eye[2] - aim_z
    off = lambda p: (p[0] + 300 * math.cos(math.radians(RANGE_YAW + 90)),
                     p[1] + 300 * math.sin(math.radians(RANGE_YAW + 90)), p[2])
    p1 = along(first, z)
    p2 = along(first + gap, z)
    fy = RANGE_YAW if first_yaw is None else first_yaw
    t1 = target(p1 if first_on else off(p1), yaw=fy, hover=1)
    t2 = targetn(1, p2 if second_on else off(off(p2)), yaw=RANGE_YAW, hover=1)
    give(weapon, 30, 0)
    aimp = along(first + gap / 2, eye[2])
    aimp = (aimp[0] + side * math.cos(math.radians(RANGE_YAW + 90)),
            aimp[1] + side * math.sin(math.radians(RANGE_YAW + 90)), aimp[2])
    pitch, yaw = angles_to(eye, aimp)
    b = IN_DUCK
    ins = idle(90, pitch, yaw, buttons=b)
    if weapon in ZOOMED:
        ins += [(b | IN_ATTACK2, 0, 0, pitch, yaw)] + idle(40, pitch, yaw, buttons=b)
    st = []
    for k in range(shots):
        st.append(len(ins) + 1)
        ins += [(b | IN_ATTACK, 0, 0, pitch, yaw)] + idle(110, pitch, yaw, buttons=b)
    r = run("coll", ins, start=(RANGE_POS[0], RANGE_POS[1], RANGE_POS[2], 0, RANGE_YAW))
    c1 = int(t1.split("target ")[1].split()[0]) if "target " in t1 else None
    c2 = int(t2.split("= client ")[1].split()[0]) if "= client " in t2 else None
    out = []
    for shot in st:
        imps = [tuple(float(x) for x in a[1:4]) for t, _, a in r.ev("bullet_impact") if t == shot]
        rows = []
        for v, h, g in hurts_by_victim(r, shot):
            pv = p1 if v == c1 else p2
            imp = min(imps, key=lambda q: math.hypot(q[0] - pv[0], q[1] - pv[1])) if imps else pv
            rows.append(("t1" if v == c1 else "t2" if v == c2 else v, h, g, round(dist(eye, imp), 1)))
        out.append((rows, [tuple(round(c, 1) for c in q) for q in imps]))
    return out


def coll_reach(weapon, first, lo, hi, eye, aim_z=55.0, tol=10.0):
    """Largest gap behind target 1 at which target 2 is still hit (bisection)."""
    while hi - lo > tol:
        mid = (lo + hi) / 2
        rows, _ = collateral(weapon, aim_z=aim_z, first=first, gap=mid, eye=eye)[0]
        if any(r[0] == "t2" for r in rows):
            lo = mid
        else:
            hi = mid
    return lo, hi


RANGE_END = 4269.0  # eye -> the wall at the end of the range line


def after_player_reach(weapon, lo, hi, eye, tol=8.0):
    """Bisection on target 1's distance: smallest distance from which a bullet
    that hit target 1 still reaches the wall at the end of the line. Returns
    the reach after the player (RANGE_END - first) bounds."""
    while hi - lo > tol:
        mid = (lo + hi) / 2
        res = collateral(weapon, first=mid, gap=100, second_on=False, eye=eye, shots=1)
        rows, imps = res[0]
        if not any(r[0] == "t1" for r in rows):
            continue
        far = any(dist(eye, q) > RANGE_END - 20 for q in imps)
        if far:
            hi = mid
        else:
            lo = mid
    return RANGE_END - hi, RANGE_END - lo


# ---------------------------------------------------------------------------
# M3 recoil for other weapons

def recoil_taps(weapon, shots=10, period=None, yaw=270.0, after=120, duck=False, buttons=0, fwd=0.0, side=0.0,
                clip=None):
    """Semi-automatic weapons: press attack every `period` ticks (default: the
    first tick the weapon can fire again) for `shots` presses."""
    give(weapon, clip or max(shots, 1), 0)
    b = (IN_DUCK if duck else 0) | buttons
    ins = idle(90, 0, yaw, buttons=b)
    for k in range(shots):
        ins += [(b | IN_ATTACK, fwd, side, 0, yaw)] + [(b, fwd, side, 0, yaw)] * (period - 1)
    ins += idle(after, 0, yaw, buttons=b)
    return run("recoil", ins, start=(SPAWN[0], SPAWN[1], SPAWN[2], 0, yaw))


def kicks(r):
    """Per shot: (n = shots fired after the shot, kick up, kick left, punch
    pitch, punch yaw after the shot, penalty before and after the shot)."""
    out = []
    for t in fire_ticks(r):
        if t - 1 not in r.w:
            continue
        before = decay_punch(r.w[t - 1]["punch"])
        after = r.w[t]["punch"]
        out.append((int(r.w[t]["shots"]), round(before[0] - after[0], 4), round(after[1] - before[1], 4),
                    round(after[0], 4), round(after[1], 4), r.w[t - 1]["penalty"], r.w[t]["penalty"], t))
    return out


def decay_err(r):
    f = set(fire_ticks(r))
    err = 0.0
    for t in sorted(r.w):
        if t - 1 in r.w and t not in f:
            pred = decay_punch(r.w[t - 1]["punch"])
            err = max(err, abs(pred[0] - r.w[t]["punch"][0]), abs(pred[1] - r.w[t]["punch"][1]))
    return err


def reach_ground(weapon, pitch, first, eye, shots=1, yaw=RANGE_YAW):
    """Shot along the range yaw, pitched down so it meets the ground at L;
    target 1 sits on the line `first` units from the eye (chest on the line).
    Returns (L, hit target 1?, impact distances)."""
    L = surfaces(eye, pitch, yaw, n=1)[0][0]
    a, p = math.radians(yaw), math.radians(pitch)
    d = (math.cos(p) * math.cos(a), math.cos(p) * math.sin(a), -math.sin(p))
    c = tuple(eye[i] + d[i] * first for i in range(3))
    target((c[0], c[1], c[2] - 55.0), yaw=yaw, hover=1)
    targetn(1, (c[0] + 600, c[1] + 600, c[2]), yaw=yaw, hover=1)
    give(weapon, 30, 0)
    b = IN_DUCK
    ins = idle(90, pitch, yaw, buttons=b)
    if weapon in ZOOMED:
        ins += [(b | IN_ATTACK2, 0, 0, pitch, yaw)] + idle(40, pitch, yaw, buttons=b)
    st = []
    for k in range(shots):
        st.append(len(ins) + 1)
        ins += [(b | IN_ATTACK, 0, 0, pitch, yaw)] + idle(110, pitch, yaw, buttons=b)
    r = run("reach", ins, start=(RANGE_POS[0], RANGE_POS[1], RANGE_POS[2], pitch, yaw))
    out = []
    for s in st:
        hit = bool(hurts_by_victim(r, s))
        imps = [round(dist(eye, tuple(float(x) for x in a_[1:4])), 1) for tt, _, a_ in r.ev("bullet_impact") if tt == s]
        out.append((hit, imps))
    return L, out


def reach_bisect(weapon, pitch, yaw, lo, hi, eye, tol=8.0):
    """Smallest target-1 distance from which the bullet still reaches the
    wall L away after passing target 1."""
    L = None
    while hi - lo > tol:
        mid = (lo + hi) / 2
        L, out = reach_ground(weapon, pitch, mid, eye, yaw=yaw)
        hit, imps = out[0]
        if not hit:
            continue
        if any(abs(d - L) < 30 for d in imps):
            hi = mid
        else:
            lo = mid
    return L, lo, hi


def wall_pass(weapon, foot, aims, duck=True, settle=60):
    """Shots from `foot` at (pitch, yaw) aims with no target: per shot, the
    layers along the bullet's actual path (eye -> its first impact, extended)
    and whether it reached the next surface (an impact beyond layer 1).
    Returns rows (pitch, yaw, layer list, impact distances, passed)."""
    target((foot[0] + 2000, foot[1], foot[2] - 3000), hover=1)
    targetn(1, (foot[0] + 2100, foot[1], foot[2] - 3000), hover=1)
    b = IN_DUCK if duck else 0
    give(weapon, 90, 0)
    ins = idle(settle, aims[0][0], aims[0][1], buttons=b)
    st = []
    for (p, y) in aims:
        ins += idle(20, p, y, buttons=b)
        if weapon in ZOOMED:
            ins += [(b | IN_ATTACK2, 0, 0, p, y)] + idle(40, p, y, buttons=b)
        st.append((len(ins) + 1, p, y))
        ins += [(b | IN_ATTACK, 0, 0, p, y)] + idle(100 if weapon in ZOOMED else 50, p, y, buttons=b)
        if weapon in ZOOMED:
            ins += [(b | IN_ATTACK2, 0, 0, p, y)] + idle(30, p, y, buttons=b) + \
                   [(b | IN_ATTACK2, 0, 0, p, y)] + idle(30, p, y, buttons=b)
    r = run("wpass", ins, start=(foot[0], foot[1], foot[2], aims[0][0], aims[0][1]), timeout=300)
    rows = []
    for (t, p, y) in st:
        eye = tuple(r.w[t]["eye"])
        imps = [tuple(float(x) for x in a[1:4]) for tt, _, a in r.ev("bullet_impact") if tt == t]
        if not imps:
            continue
        ap, ay = angles_to(eye, imps[0])
        lay = surfaces(eye, ap, ay, n=3)
        ds = [round(dist(eye, q), 1) for q in imps]
        passed = len(lay) > 1 and any(abs(d - lay[1][0]) < 3 for d in ds[1:])
        rows.append((p, y, round(ap, 3), round(ay, 3), lay, ds, passed))
    return rows


def aim_catalog(eye, yaws, pitches, mat_props=None, tmin=0.0, tmax=64.0, next_max=600.0):
    """(thickness, pitch, yaw, layer1, layer2) for aims whose first layer has
    the given surface props and a next surface to show an impact."""
    out = []
    for y in yaws:
        for p in pitches:
            l = surfaces(eye, p, y, n=2)
            if len(l) < 2:
                continue
            if mat_props is not None and l[0][2] not in mat_props:
                continue
            if not (tmin <= l[0][1] <= tmax):
                continue
            if l[1][0] - l[0][0] - l[0][1] > next_max or l[1][0] - l[0][0] - l[0][1] < 2:
                continue
            out.append((l[0][1], p, y, l[0], l[1]))
    out.sort()
    return out


def multi(weapon, foot, yaw, pitch, targets, shots=2, duck=True):
    """Shots along (pitch, yaw) from `foot` with targets (backs to the shooter)
    whose chest (origin + 55) sits on the line at the given distances from
    the eye (None: parked). Returns per shot (hurt rows (victim, dmg, group),
    impact distances) and the layers along the line."""
    b = IN_DUCK if duck else 0
    r0 = run("pen_eye", idle(30, 0, 0, buttons=b), start=(foot[0], foot[1], foot[2], 0, 0))
    eye = tuple(r0.w[30]["eye"])
    a, p = math.radians(yaw), math.radians(pitch)
    d = (math.cos(p) * math.cos(a), math.cos(p) * math.sin(a), -math.sin(p))
    clients = []
    for k, td in enumerate(targets):
        if td is None:
            pos = (foot[0] + 3000 + 100 * k, foot[1], foot[2] - 3000)
        else:
            c = tuple(eye[i] + d[i] * td for i in range(3))
            pos = (c[0], c[1], c[2] - 55.0)
        out = target(pos, yaw=yaw, hover=1) if k == 0 else targetn(k, pos, yaw=yaw, hover=1)
        clients.append(out)
    give(weapon, 30, 0)
    ins = idle(60, pitch, yaw, buttons=b)
    if weapon in ZOOMED:
        ins += [(b | IN_ATTACK2, 0, 0, pitch, yaw)] + idle(40, pitch, yaw, buttons=b)
    st = []
    for k in range(shots):
        st.append(len(ins) + 1)
        ins += [(b | IN_ATTACK, 0, 0, pitch, yaw)] + idle(110, pitch, yaw, buttons=b)
    r = run("multi", ins, start=(foot[0], foot[1], foot[2], pitch, yaw))
    res = []
    for s in st:
        imps = [round(dist(eye, tuple(float(x) for x in a_[1:4])), 1) for tt, _, a_ in r.ev("bullet_impact") if tt == s]
        res.append((hurts_by_victim(r, s), imps))
    return res, surfaces(eye, pitch, yaw, n=4), eye


def kick_fit(runs):
    """Fit the per-shot kick from several recoil runs: up = a (n = 1),
    a + b n (n > 1); side likewise; caps from the largest |punch|; flips =
    side sign changes between consecutive shots. Shots whose punch hit a cap
    are skipped in the check. Returns a dict and the worst residual."""
    ks = [kicks(r) for r in runs]
    first = [k for k in ks if k and k[0][0] == 1]
    a_up = first[0][0][1]
    a_sd = abs(first[0][0][2])
    seconds = [x for k in ks for x in k if x[0] == 2]
    b_up = (seconds[0][1] - a_up) / 2 if seconds else None
    b_sd = (abs(seconds[0][2]) - a_sd) / 2 if seconds else None
    cap_p = min(x[3] for k in ks for x in k)
    cap_y = max(abs(x[4]) for k in ks for x in k)
    worst = 0.0
    flips = pairs = 0
    for k in ks:
        prev = None
        for x in k:
            n = x[0]
            if b_up is None:
                break
            if n == 0:  # dry fire
                continue
            up = a_up if n == 1 else a_up + b_up * n
            sd = a_sd if n == 1 else a_sd + b_sd * n
            if x[3] > cap_p + 1e-3:
                worst = max(worst, abs(x[1] - up))
            if abs(x[4]) < cap_y - 1e-3:
                worst = max(worst, abs(abs(x[2]) - sd))
            if prev is not None and n == prev[0] + 1:
                pairs += 1
                if (x[2] > 0) != (prev[2] > 0):
                    flips += 1
            prev = x
    return dict(up=(a_up, b_up), side=(a_sd, b_sd), cap_pitch=cap_p, cap_yaw=cap_y, flips=flips, pairs=pairs,
                worst=round(worst, 4))


def recoil_state(weapon="weapon_ak47", state="run", shots=12, yaw=270.0, walk_speed=None):
    """Hold attack while moving sideways (`run`: 250, `walk`: IN_SPEED, or a
    given speed) or right after a jump (`air`: attack held from the tick after
    the jump for 40 ticks). Returns the run; kicks() gives the per-shot kick
    and r.ticks the speed/ground state."""
    give(weapon, shots, 0)
    ins = idle(90, 0, yaw)
    if state == "air":
        ins += [(IN_JUMP, 0, 0, 0, yaw)] + [(IN_ATTACK, 0, 0, 0, yaw)] * 40 + idle(60, 0, yaw)
    else:
        b = IN_SPEED if state == "walk" else 0
        v = walk_speed or 250.0
        ins += [(b, 0, v, 0, yaw)] * 40 + [(b | IN_ATTACK, 0, v, 0, yaw)] * (shots * 7) + idle(60, 0, yaw)
    return run("recoil_state", ins, start=(SPAWN[0], SPAWN[1], SPAWN[2], 0, yaw))


def kick_vs_speed(weapon="weapon_ak47", delays=range(0, 24, 2), yaw=270.0):
    """One shot `d` ticks after releasing a full-speed side run (speed
    falling by friction): first-shot kick vs. the speed on the shot tick."""
    out = []
    for d in delays:
        give(weapon, 3, 0)
        ins = idle(90, 0, yaw) + [(0, 0, 250.0, 0, yaw)] * 40 + idle(d, 0, yaw) + [(IN_ATTACK, 0, 0, 0, yaw)] + \
            idle(30, 0, yaw)
        r = run("kick_speed", ins, start=(SPAWN[0], SPAWN[1], SPAWN[2], 0, yaw))
        for x in kicks(r)[:1]:
            t = x[7]
            out.append((d, round(math.hypot(r.ticks[t][4], r.ticks[t][5]), 2),
                        round(math.hypot(r.ticks[t - 1][4], r.ticks[t - 1][5]), 2), x[1], x[2]))
    return out
