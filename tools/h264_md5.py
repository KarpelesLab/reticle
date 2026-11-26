#!/usr/bin/env python3
"""Writes the per-frame md5 list of a raw I420 file decoded from a stream.

Usage: h264_md5.py STREAM.264 FRAMES.yuv OUT.md5

The picture size comes from ffprobe on the stream (the cropped size). Each
output line is "<frame index> <md5 hex>"; the first line is a comment with
the size and the frame count.
"""
import hashlib
import subprocess
import sys


def main():
    src, yuv, md5 = sys.argv[1:4]
    probe = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
         "stream=width,height", "-of", "csv=p=0", src],
        check=True, capture_output=True, text=True).stdout.strip().split(",")
    w, h = int(probe[0]), int(probe[1])
    size = w * h + 2 * ((w + 1) // 2) * ((h + 1) // 2)
    with open(yuv, "rb") as f:
        data = f.read()
    if len(data) % size:
        sys.exit("%s: %d bytes is not a whole number of %dx%d frames"
                 % (yuv, len(data), w, h))
    n = len(data) // size
    with open(md5, "w") as f:
        f.write("# %dx%d %d frames\n" % (w, h, n))
        for i in range(n):
            f.write("%d %s\n" % (i, hashlib.md5(data[i * size:(i + 1) * size]).hexdigest()))


if __name__ == "__main__":
    main()
