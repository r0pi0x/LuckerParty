"""Minimal VPK (v1/v2) directory reader: list and extract files from the
user's own install for measurement only (nothing extracted is committed).

python3 tools/css_probe/vpkls.py <pak_dir.vpk> [substring] [--cat path]
"""

import os
import struct
import sys


def read_dir(path):
    f = open(path, "rb")
    sig, ver, tree = struct.unpack("<III", f.read(12))
    assert sig == 0x55AA1234, "not a vpk"
    if ver == 2:
        f.read(16)
    hdr = f.tell()

    def cstr():
        b = b""
        while True:
            c = f.read(1)
            if c == b"\0":
                return b.decode(errors="replace")
            b += c

    entries = {}
    while True:
        ext = cstr()
        if not ext:
            break
        while True:
            d = cstr()
            if not d:
                break
            while True:
                n = cstr()
                if not n:
                    break
                crc, pre, arc, off, ln, term = struct.unpack("<IHHIIH", f.read(18))
                predata = f.read(pre)
                full = (d.strip() + "/" if d.strip() else "") + n + "." + ext
                entries[full] = (arc, off, ln, predata, hdr + tree)
    return entries


def cat(dirpath, entries, name):
    arc, off, ln, pre, dataoff = entries[name]
    if arc == 0x7FFF:
        with open(dirpath, "rb") as f:
            f.seek(dataoff + off)
            return pre + f.read(ln)
    p = dirpath.replace("_dir.vpk", "_%03d.vpk" % arc)
    with open(p, "rb") as f:
        f.seek(off)
        return pre + f.read(ln)


if __name__ == "__main__":
    d = os.path.expanduser(sys.argv[1])
    e = read_dir(d)
    if "--cat" in sys.argv:
        sys.stdout.write(cat(d, e, sys.argv[sys.argv.index("--cat") + 1]).decode(errors="replace"))
    else:
        sub = sys.argv[2] if len(sys.argv) > 2 else ""
        for k in sorted(e):
            if sub in k:
                print(k)
