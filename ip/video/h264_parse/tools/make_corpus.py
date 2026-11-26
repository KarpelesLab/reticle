#!/usr/bin/env python3
"""Writes one structural corpus for tests/ip_h264_parse.rs: an H.264 stream
made by ffmpeg, and what ffmpeg's own decoder says about every macroblock
of it.

    python3 ip/video/h264_parse/tools/make_corpus.py NAME 'ENCODER ARGS' > testdata/ip/h264_parse_NAME.txt

ENCODER ARGS are ffmpeg output options, e.g.
    '-f lavfi -i testsrc2=size=176x144:rate=25 -frames:v 4 -c:v libx264 -profile:v baseline'
(the input options go first; the stream is written as raw Annex B).

ffmpeg is run as a black box: its encoder makes the stream, and its
decoder's `-debug mb_type` and `-debug qp` maps are the expected answer.
Those two maps are all that is recorded about each macroblock -- a class
and QPY -- because they are all ffmpeg prints; the token-exact check is
against the software model's dumps.

Format:
    # comments
    stream <hex>            the bytes, in order, over as many lines as needed
    pic <width_mbs> <height_mbs>
    mb <class> <qp>         one per macroblock, raster order, picture by picture
with class one of: i (I_NxN), I (I_16x16), S (P_Skip), P16 (P_L0_16x16),
P168, P816, P8 (P_8x8 or P_8x8ref0), PCM.
"""
import os
import re
import shlex
import subprocess
import sys
import tempfile

CLASSES = {'i': 'i', 'I': 'I', 'S': 'S', '>': 'P16', '>-': 'P168', '>|': 'P816', '>+': 'P8', 'P': 'PCM'}


def ff_maps(path, what):
    out = subprocess.run(['ffmpeg', '-hide_banner', '-threads', '1', '-debug', what, '-i', path,
                          '-f', 'null', '-'], capture_output=True, text=True, check=True).stderr
    frames, cur = [], None
    for line in out.split('\n'):
        m = re.match(r'\[h264 @ [^\]]*\] (.*)$', line)
        if not m:
            continue
        t = m.group(1)
        if t.startswith('New frame'):
            cur = []
            frames.append(cur)
            continue
        if cur is None:
            continue
        if not re.fullmatch(r'[ 0-9A-Za-z<>|+\-=X]*', t) or t.startswith('nal_unit_type'):
            cur = None
            continue
        cur.append(t)
    return frames


def cells(lines, w):
    return [[l[i:i + w] for i in range(0, len(l), w)] for l in lines]


def main():
    name, args = sys.argv[1], sys.argv[2]
    with tempfile.TemporaryDirectory() as d:
        if args.startswith('file:'):
            # A stream made some other way (make_pcm.py): file:PATH:PICTURES.
            _, path, count = args.split(':')
            count = int(count)
            how = 'by %s' % os.path.basename(path)
        else:
            path = os.path.join(d, name + '.264')
            subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y'] + shlex.split(args)
                           + ['-bsf:v', 'filter_units=remove_types=6', '-f', 'h264', path], check=True)
            m = re.search(r'-frames:v (\d+)', args)
            count = int(m.group(1))
            how = 'by `ffmpeg %s -bsf:v filter_units=remove_types=6 -f h264`' % args
        data = open(path, 'rb').read()
        types = ff_maps(path, 'mb_type')
        qps = ff_maps(path, 'qp')
    out = sys.stdout
    out.write('# %s: made %s\n' % (name, how))
    out.write('# macroblock classes and QPY from ffmpeg -debug mb_type / -debug qp\n')
    for i in range(0, len(data), 32):
        out.write('stream %s\n' % data[i:i + 32].hex())
    # The probe decodes some pictures before the real pass, so the real
    # pass is the trailing `count` of them.
    types = types[len(types) - count:]
    qps = qps[len(qps) - count:]
    for t, q in zip(types, qps):
        tc = cells(t, 3)
        qc = cells(q, 2)
        h = len(tc)
        w = len(qc[0])
        out.write('pic %d %d\n' % (w, h))
        for y in range(h):
            for x in range(w):
                out.write('mb %s %d\n' % (CLASSES[tc[y][x].strip()], int(qc[y][x])))


if __name__ == '__main__':
    main()
