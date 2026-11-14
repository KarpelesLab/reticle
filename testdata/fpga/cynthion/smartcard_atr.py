#!/usr/bin/env python3
"""Power a card through smartcard_serial.v, print its ATR, and power it down.

    smartcard_atr.py /dev/ttyACM1 [--pps[=PPS1]] [seconds]

Raising DTR runs the bridge's activation sequence; dropping it deactivates.
The port's baud rate is the card's bit rate: 7.5 MHz / 372 = 20161 at ATR.
With --pps, a PPS asks for the ATR's own TA1, as a Ledger Nano S's MCU does
(FF 10 98 77 for TA1 = 98); --pps=87 asks for F/D = 4, 1.875 Mbaud here, a
value Ledger's secure elements take although ISO 7816 reserves Fi index 8.
On a matching echo the port moves to the new rate and the card is held up for
the rest of the time. Needs pyserial.
"""
import sys
import time

import serial

CARD_HZ = 7_500_000
# Clock cycles per bit for PPS1 values ISO 7816 leaves reserved but Ledger's
# secure elements accept (docs/card-bench.md: 87 is F/D = 4).
VENDOR_FD = {0x87: 4}
FI = [372, 372, 558, 744, 1116, 1488, 1860, None, None, 512, 768, 1024, 1536, 2048, None, None]
DI = [None, 1, 2, 4, 8, 16, 32, 64, 12, 20, None, None, None, None, None, None]


def decode_atr(atr):
    """Print the ATR's interface bytes, as far as they go."""
    if not atr:
        print("no ATR: the card sent nothing")
        return
    ts = atr[0]
    print(f"TS = {ts:02X}", {0x3B: "(direct)", 0x3F: "(inverse)"}.get(ts, "(not a TS byte)"))
    if len(atr) < 2:
        return
    i, y, k = 1, atr[1] >> 4, atr[1] & 0x0F
    print(f"T0 = {atr[1]:02X}: {k} historical byte(s)")
    level, protocols = 1, []
    i = 2
    while True:
        for bit, name in ((0x1, "TA"), (0x2, "TB"), (0x4, "TC"), (0x8, "TD")):
            if y & bit:
                if i >= len(atr):
                    print("  (ATR ends early)")
                    return
                v = atr[i]
                note = ""
                if name == "TA" and level == 1:
                    fi, di = FI[v >> 4], DI[v & 0x0F]
                    if fi and di:
                        note = (f"  Fi={fi} Di={di}: {CARD_HZ * di / fi:.0f} baud once "
                                f"negotiated")
                print(f"  {name}{level} = {v:02X}{note}")
                i += 1
                if name == "TD":
                    protocols.append(v & 0x0F)
                    y = v >> 4
                    level += 1
                    break
        else:
            break
    hist = atr[i:i + k]
    print(f"historical: {hist.hex(' ')}  {bytes(c if 32 <= c < 127 else 46 for c in hist).decode()}")
    if protocols:
        print("protocols offered: " + ", ".join(f"T={p}" for p in protocols))


def read_for(s, secs):
    data = bytearray()
    end = time.time() + secs
    while time.time() < end:
        data += s.read(256)
    return bytes(data)


def ta1_of(atr):
    """TA1, if the ATR has one."""
    return atr[2] if len(atr) > 2 and atr[1] & 0x10 else None


def pps(s, atr, ask=None):
    ta1 = ask if ask is not None else ta1_of(atr)
    if ta1 is None:
        print("PPS: the ATR offers no TA1; staying at the ATR's rate")
        return
    fi, di = FI[ta1 >> 4], DI[ta1 & 0x0F]
    if ta1 in VENDOR_FD:
        fi, di = VENDOR_FD[ta1], 1
    req = bytes([0xFF, 0x10, ta1])
    req += bytes([req[0] ^ req[1] ^ req[2]])
    s.write(req)
    echo = read_for(s, 0.2)
    print(f"PPS sent {req.hex(' ')}, echo {echo.hex(' ') or '(nothing)'}")
    if echo != req or not (fi and di):
        print("PPS: no matching echo; staying at the ATR's rate")
        return
    s.baudrate = round(CARD_HZ * di / fi)
    print(f"PPS accepted: port now {s.baudrate} baud (Fi={fi} Di={di})")


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    port = args[0]
    secs = float(args[1]) if len(args) > 1 else 2.0
    s = serial.Serial(port, baudrate=round(CARD_HZ / 372), parity=serial.PARITY_EVEN,
                      timeout=0.1, dsrdtr=False, rtscts=False)
    s.dtr = False
    time.sleep(0.2)
    s.reset_input_buffer()
    s.dtr = True                       # activate
    atr = read_for(s, 0.5)
    print("ATR:", atr.hex(" ") if atr else "(nothing)")
    decode_atr(atr)
    for a in sys.argv[1:]:
        if a == "--pps":
            pps(s, atr)
        elif a.startswith("--pps="):
            pps(s, atr, int(a.split("=", 1)[1], 16))
    rest = read_for(s, max(0.0, secs - 0.5))
    if rest:
        print("then:", rest.hex(" "))
    print(f"DCD (card running) = {s.cd}")
    s.dtr = False                      # deactivate
    s.close()


if __name__ == "__main__":
    main()
