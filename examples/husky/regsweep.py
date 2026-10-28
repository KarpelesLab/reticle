"""Checks that a Husky build's registers store what is written to them.

    python regsweep.py

Run against whatever bitstream is loaded, through NewAE's `chipwhisperer`
package. Writes zero, all ones and a walking one into every bit of ten of
NewAE's plain read/write registers, reads each back, and prints, per
register, the bits that read one when written zero and zero when written
one. Exits non-zero if any register is wrong.

The registers are ones that drive nothing outside the FPGA: `ECHO`, the
capture lengths and offsets, the segment counts, the ADC trigger level,
the logic analyser's depth and downsampling, and the four target I/O
bytes of `CW_IOROUTE` (its bytes 4 and 5 switch the glitch outputs and
the target's power, and are left at zero). Nothing is attached to the
target port when this runs, and everything it writes is volatile: loading
a bitstream again resets it.

A build whose placement broke a register's storage or its read path
shows it here in seconds, where NewAE's suite takes 25 minutes and may
stop at its first test. It found a long line driven from both ends of
one net (`docs/fpga-xray.md`, "One wire, two drivers").
"""

import sys

import chipwhisperer as cw
from chipwhisperer.capture.scopes._OpenADCInterface import CODE_READ, CODE_WRITE

REGISTERS = [
    ("ECHO_ADDR", 64),
    ("DECIMATE_ADDR", 16),
    ("PRESAMPLES_ADDR", 17),
    ("OFFSET_ADDR", 32),
    ("NUM_SEGMENTS", 16),
    ("SEGMENT_CYCLES", 20),
    ("ADC_TRIGGER_LEVEL", 12),
    ("LA_CAPTURE_DEPTH", 32),
    ("LA_DOWNSAMPLE", 16),
]


def sweep(sc, name, bits, size=None, pad=0):
    """Stuck bits of one register: (read one when written zero, read zero
    when written one). `size` bytes are read; `pad` zero bytes follow
    every write."""
    n = (bits + 7) // 8
    size = size or n
    mask = (1 << bits) - 1
    ones = zeros = 0
    for value in [0, mask] + [1 << b for b in range(bits)]:
        sc.sendMessage(CODE_WRITE, name, list(value.to_bytes(n, "little")) + [0] * pad, Validate=False)
        got = int.from_bytes(bytes(sc.sendMessage(CODE_READ, name, maxResp=size))[:n], "little") & mask
        ones |= got & ~value
        zeros |= value & ~got
    return ones, zeros


def main():
    scope = cw.scope()
    sc = scope.sc
    wrong = 0
    checks = [(name, bits, None, 0) for name, bits in REGISTERS]
    checks.append(("CW_IOROUTE_ADDR", 32, 8, 4))
    for name, bits, size, pad in checks:
        ones, zeros = sweep(sc, name, bits, size, pad)
        if ones or zeros:
            wrong += 1
            print(f"{name:18} {bits:3} bits: reads 1 when 0: {ones:#x}; reads 0 when 1: {zeros:#x}")
        else:
            print(f"{name:18} {bits:3} bits: ok")
    # CW_IOROUTE's reset value: TIO1 as serial transmit, TIO2 receive.
    sc.sendMessage(CODE_WRITE, "CW_IOROUTE_ADDR", [1, 2, 0, 0, 0, 0, 0, 0], Validate=False)
    print("registers wrong:", wrong)
    return 1 if wrong else 0


if __name__ == "__main__":
    sys.exit(main())
