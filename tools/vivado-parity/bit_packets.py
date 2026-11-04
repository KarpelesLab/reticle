#!/usr/bin/env python3
"""Prints the configuration packets of a 7-series `.bit` or `.bin`, one per line.

    bit_packets.py <file>

Everything but the frame data, which a line says the length and a hash of: the
dumps under `testdata/fpga/vivado/bitstream/` are this script's output, and they
are answers about a stream rather than a copy of one. The format is the one
`tests/fpga_xray_bitstream.rs` reads:

    leading ffffffff*8 / 000000bb / 11220044 / ffffffff*2 / aa995566
    nop 1
    CMD 00000007 RCRC
    FDRI type2 word(s)=<n> fnv=<64-bit hash>
    FDRI type1 word(s)=<n> fnv=<64-bit hash>

Reading: `FDRI` is written once per frame range, and each line is one packet, so
a stream that uses `MFWR` shows it as what it is. CRC values are printed as they
stand, and whether they are right is checked by `read_bit`, not here.
"""
import sys

REGS = {
    0: "CRC", 1: "FAR", 2: "FDRI", 3: "FDRO", 4: "CMD", 5: "CTL0", 6: "MASK",
    7: "STAT", 8: "LOUT", 9: "COR0", 10: "MFWR", 11: "CBC", 12: "IDCODE",
    13: "AXSS", 14: "COR1", 16: "WBSTAR", 17: "TIMER", 19: "RSVD19",
    22: "BOOTSTS", 24: "CTL1", 31: "BSPI",
}
CMDS = {
    0: "NULL", 1: "WCFG", 2: "MFW", 3: "LFRM", 4: "RCFG", 5: "START", 7: "RCRC",
    8: "AGHIGH", 9: "SWITCH", 10: "GRESTORE", 11: "SHUTDOWN", 13: "DESYNC",
    15: "IPROG",
}


def fnv(words):
    """FNV-1a over the big-endian bytes: enough to tell frame data apart."""
    h = 0xcbf29ce484222325
    for w in words:
        for b in w.to_bytes(4, "big"):
            h = ((h ^ b) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return h


def payload(data):
    """The configuration stream: what is left when the `.bit` header is gone."""
    if data[:2] != b"\x00\x09":
        return data  # a .bin, which is the same thing with no header
    i = 2 + 9 + 2
    while True:
        key = data[i]
        i += 1
        if key == ord("e"):
            n = int.from_bytes(data[i:i + 4], "big")
            return data[i + 4:i + 4 + n]
        n = int.from_bytes(data[i:i + 2], "big")
        i += 2 + n


def dump(data):
    out = []
    data = payload(data)
    sync = data.find(bytes.fromhex("aa995566"))
    lead = [int.from_bytes(data[i:i + 4], "big") for i in range(0, sync, 4)]
    parts = []
    for w in lead:
        if parts and parts[-1][0] == w:
            parts[-1][1] += 1
        else:
            parts.append([w, 1])
    text = " / ".join(f"{w:08x}" + (f"*{n}" if n > 1 else "") for w, n in parts)
    out.append(f"leading {text} / aa995566")
    words = [int.from_bytes(data[i:i + 4], "big") for i in range(sync + 4, len(data), 4)]
    i = 0
    nops = 0
    last = 0
    def flush():
        nonlocal nops
        if nops:
            out.append(f"nop {nops}")
        nops = 0
    while i < len(words):
        h = words[i]
        i += 1
        kind = h >> 29
        op = (h >> 27) & 3
        if kind == 1:
            reg = (h >> 13) & 0x3FFF
            n = h & 0x7FF
            last = reg
        elif kind == 2:
            reg = last
            n = h & 0x7FFFFFF
        else:
            flush()
            out.append(f"word {h:08x} (not a packet header)")
            continue
        body = words[i:i + n]
        i += n
        if op == 0:
            nops += 1
            continue
        flush()
        name = REGS.get(reg, f"REG{reg}")
        if op == 1:
            out.append(f"read {name} {n}")
        elif reg in (2, 3):
            out.append(f"{name} type{kind} words={n} fnv={fnv(body):016x}")
        elif reg == 4 and n == 1:
            out.append(f"CMD {body[0]:08x} {CMDS.get(body[0], '?')}")
        else:
            values = " ".join(f"{w:08x}" for w in body)
            out.append(f"{name} {values}".rstrip())
    flush()
    return out


if __name__ == "__main__":
    with open(sys.argv[1], "rb") as f:
        print("\n".join(dump(f.read())))
