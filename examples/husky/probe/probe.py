"""Checks husky_probe.bit on an attached ChipWhisperer-Husky Plus.

    python probe.py <husky_probe.bit> [<NewAE's cwhusky_plus_top.bit>]

Loads the probe through NewAE's `chipwhisperer` package (a volatile load:
reconnecting with NewAE's software puts their gateware back), then over the
USB register bus:

  addr 4   ECHO: written and read back
  addr 10  VERSION: reads (2, 88), what a Husky Plus reports
  addr 60  A, addr 61 B: 32-bit operands, written
  addr 62  A + B, addr 64 A - B, addr 65 A < B: registered, read back and
           compared with Python's arithmetic for edge cases and 300 random
           pairs (carry chains)
  addr 63  a snapshot of a free-running 32-bit counter, taken on a write
  addr 66  that counter, live; addr 67 an 8-bit one
  addr 70  a counter on the board PLL's clock to the FPGA (PLLFPGAP/N)
  addr 71  a counter on the ADC's clock back (ADC_clk_fbp/n)

The board's PLL is configured by NewAE's gateware and software, not by
the probe: given NewAE's bitstream as the second argument, the script
connects with it first, asks for a 10 MHz clock, and then swaps the FPGA's
configuration for the probe's over the same link, so that the PLL counter
has something to count. The ADC counter reads zero either way: the ADC's
control pins are not driven by the probe.

Prints what it found and exits non-zero on any wrong answer.
"""

import random
import sys
import time

import chipwhisperer as cw
from chipwhisperer.capture.scopes.OpenADC import OpenADC
from chipwhisperer.capture.scopes._OpenADCInterface import CODE_READ, CODE_WRITE


def connect(probe, reference):
    if reference:
        scope = cw.scope(bitstream=reference, force=True)
        scope.clock.clkgen_src = "system"
        scope.clock.clkgen_freq = 10e6
        scope.clock.adc_mul = 1
        time.sleep(0.5)
        with open(probe, "rb") as f:
            scope.scopetype.fpga.FPGAProgram(f, prog_speed=10e6)
        return scope.sc, scope.dis
    scope = OpenADC()
    try:
        scope.con(bitstream=probe, force=True)
    except Exception:
        # NewAE's connect goes on to set up a whole scope the probe is not.
        pass
    return scope.sc, scope.scopetype.ser.close


def main():
    sc, close = connect(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None)
    rd = lambda a, n: bytes(sc.sendMessage(CODE_READ, a, maxResp=n))
    wr = lambda a, d: sc.sendMessage(CODE_WRITE, a, bytearray(d))
    rd32 = lambda a: int.from_bytes(rd(a, 4), "little")
    wr32 = lambda a, v: wr(a, v.to_bytes(4, "little"))
    bad = 0

    version = list(rd(10, 2))
    print("VERSION", version)
    bad += version != [2, 88]
    for v in (0x5A, 0xA5, 0x00, 0xFF, 0x3C):
        wr(4, [v])
        if rd(4, 1)[0] != v:
            print(f"ECHO wrote {v:#04x} read {rd(4, 1)[0]:#04x}")
            bad += 1

    wr(63, [0])
    a = rd32(63)
    time.sleep(0.05)
    wr(63, [0])
    b = rd32(63)
    print(f"counter: {(b - a) & 0xFFFFFFFF} counts between snapshots ~50 ms apart")
    bad += not 3e6 < ((b - a) & 0xFFFFFFFF) < 2e7

    random.seed(1)
    cases = [(0, 0), (1, 1), (0xFFFFFFFF, 1), (0x7FFFFFFF, 1), (0x80000000, 0x80000000),
             (0x12345678, 0x9ABCDEF0), (5, 7), (7, 5), (0xFFFF, 1)]
    cases += [(random.getrandbits(32), random.getrandbits(32)) for _ in range(300)]
    wrong = 0
    for x, y in cases:
        wr32(60, x)
        wr32(61, y)
        got = (rd32(62), rd32(64), rd(65, 1)[0])
        want = ((x + y) & 0xFFFFFFFF, (x - y) & 0xFFFFFFFF, int(x < y))
        if got != want:
            wrong += 1
            if wrong <= 10:
                print(f"a={x:#010x} b={y:#010x}: got {got}, want {want}")
    print(f"arithmetic: {len(cases) - wrong} of {len(cases)} right")
    bad += wrong

    for name, addr in (("usb", 66), ("pll", 70), ("adc", 71)):
        a = rd32(addr)
        time.sleep(0.1)
        b = rd32(addr)
        print(f"{name} clock: {((b - a) & 0xFFFFFFFF) * 10:.4e} Hz")
    close()
    print("PASS" if bad == 0 else f"FAIL ({bad})")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
