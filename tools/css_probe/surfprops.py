"""Surface property index -> (name, game material letter, base) for the CS:S
server install, in the order the physics system loads them (the manifest's
files in order, new names appended; a repeated name overrides in place).
Reads the user's own install at run time; nothing is committed.

python3 tools/css_probe/surfprops.py [index ...]
"""

import os
import re
import sys

sys.path.insert(0, os.path.dirname(__file__))
import vpkls  # noqa: E402

DS = os.path.expanduser("~/games/css-ds")


def _read(rel):
    for vdir in (DS + "/cstrike/cstrike_pak_dir.vpk", DS + "/hl2/hl2_misc_dir.vpk"):
        e = vpkls.read_dir(vdir)
        if rel in e:
            return vpkls.cat(vdir, e, rel).decode(errors="replace")
    for loose in (DS + "/cstrike/" + rel, DS + "/hl2/" + rel):
        if os.path.exists(loose):
            return open(loose, errors="replace").read()
    raise FileNotFoundError(rel)


def _kv_blocks(text):
    text = re.sub(r"//[^\n]*", "", text)
    toks = re.findall(r'"([^"]*)"|([{}])|([^\s{}"]+)', text)
    toks = [a or b or c for a, b, c in toks]
    i = 0
    while i < len(toks):
        name = toks[i]
        assert toks[i + 1] == "{", (name, toks[i + 1])
        i += 2
        kv = {}
        while toks[i] != "}":
            kv[toks[i].lower()] = toks[i + 1]
            i += 2
        i += 1
        yield name.lower(), kv


def table():
    man = _read("scripts/surfaceproperties_manifest.txt")
    files = re.findall(r'"file"\s+"([^"]+)"', re.sub(r"//[^\n]*", "", man))
    order = []
    props = {}
    for f in files:
        for name, kv in _kv_blocks(_read(f)):
            if name not in props:
                order.append(name)
                props[name] = {}
            props[name].update(kv)

    def mat(n, depth=0):
        p = props.get(n, {})
        if "gamematerial" in p:
            return p["gamematerial"]
        if "base" in p and depth < 10:
            return mat(p["base"].lower(), depth + 1)
        return "C"

    return [(n, mat(n), props[n].get("base", "")) for n in order]


if __name__ == "__main__":
    t = table()
    idx = [int(a) for a in sys.argv[1:]] or range(len(t))
    for i in idx:
        print(i, *t[i])
