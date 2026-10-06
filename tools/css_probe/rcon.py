"""Minimal Source RCON client for the CS:S probe server.

Usage as a script: python3 tools/css_probe/rcon.py "<command>" [...]
"""

import socket
import struct
import sys

ADDR = ("127.0.0.1", 27030)
PASSWORD = "mashup-probe"


class Rcon:
    def __init__(self, addr=ADDR, password=PASSWORD, timeout=10.0):
        self.s = socket.create_connection(addr, timeout=timeout)
        self.next_id = 1
        self._send(3, password)
        # Auth answers with an empty response value, then the auth response.
        while True:
            rid, rtype, _ = self._recv()
            if rtype == 2:
                if rid == -1:
                    raise RuntimeError("rcon auth failed")
                break

    def _send(self, kind, body):
        rid = self.next_id
        self.next_id += 1
        data = struct.pack("<ii", rid, kind) + body.encode() + b"\0\0"
        self.s.sendall(struct.pack("<i", len(data)) + data)
        return rid

    def _read(self, n):
        buf = b""
        while len(buf) < n:
            chunk = self.s.recv(n - len(buf))
            if not chunk:
                raise RuntimeError("rcon connection closed")
            buf += chunk
        return buf

    def _recv(self):
        (size,) = struct.unpack("<i", self._read(4))
        data = self._read(size)
        rid, rtype = struct.unpack("<ii", data[:8])
        return rid, rtype, data[8:-2].decode(errors="replace")

    def exec(self, cmd):
        rid = self._send(2, cmd)
        # An empty response-value packet marks the end of a multi-packet answer.
        marker = self._send(0, "")
        out = []
        while True:
            got, _, body = self._recv()
            if got == marker:
                break
            if got == rid:
                out.append(body)
        # SRCDS answers the empty marker packet twice (an empty body, then
        # 00 01 00 00); read the second one too.
        self._recv()
        return "".join(out)


if __name__ == "__main__":
    r = Rcon()
    for c in sys.argv[1:]:
        print(r.exec(c))
