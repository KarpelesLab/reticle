#!/usr/bin/env python3
"""Whether a 7-series .bit's frames hold the Hamming code of their own contents.

    tools/vivado-parity/frame_ecc.py <file.bit> [...]

Word 50 of a 101-word frame keeps its low 13 bits for this. The formula is the
one Project X-Ray's `lib/xilinx/xc7series/ecc.cc` has, written from memory of it
and judged by whether it reproduces what Vivado wrote: this prints how many
frames agree. Frames are counted from the first long write to FDRI, so the pad
frames at each row's end are in the sequence, and hold zeros.
"""
import struct
import sys


def frames_of(path):
    data = open(path, "rb").read()
    at = data.find(b"\xaa\x99\x55\x66")
    words = struct.unpack(f">{(len(data) - at) // 4}I", data[at:at + (len(data) - at) // 4 * 4])
    i = 1
    while i < len(words):
        head = words[i]
        i += 1
        kind = head >> 29
        if kind == 1:
            op = (head >> 27) & 3
            reg = (head >> 13) & 0x3FFF
            count = head & 0x7FF
            if count == 0 and reg == 2 and op == 2 and words[i] >> 29 == 2:
                count = words[i] & 0x7FFFFFF
                body = words[i + 1:i + 1 + count]
                return [body[k:k + 101] for k in range(0, len(body) - 100, 101)]
            i += count
        elif kind == 2:
            i += head & 0x7FFFFFF
    return []


def ecc(frame):
    code = 0
    for index, word in enumerate(frame):
        val = index * 32 + (0x1360 if index > 0x25 else 0x1340 if index > 6 else 0x1320)
        if index == 0x32:
            word &= 0xFFFFE000
        for bit in range(32):
            if word >> bit & 1:
                code ^= val + bit
    v = code & 0xFFF
    v ^= v >> 8
    v ^= v >> 4
    v ^= v >> 2
    v ^= v >> 1
    return code ^ ((v & 1) << 12)


for path in sys.argv[1:]:
    frames = frames_of(path)
    agree = sum((f[50] & 0x1FFF) == ecc(f) for f in frames)
    zero_word = sum((f[50] & 0x1FFF) == 0 for f in frames)
    nonzero = sum(any(f) for f in frames)
    print(f"{path}: {len(frames)} frames, {nonzero} with a set bit; low 13 bits of word 50 are the code "
          f"of the frame in {agree}, are zero in {zero_word}")
