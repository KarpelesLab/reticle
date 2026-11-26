#!/usr/bin/env bash
#
# Regenerates the committed H.264 model dumps, testdata/ip/h264_*.txt, from
# the committed streams in testdata/ip/h264/. tests/h264_model.rs runs the
# same commands into a scratch directory and requires identical files, so a
# change to the model that changes a dump fails until this is re-run.
#
# Usage: tools/h264_dumps.sh [outdir]

set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-testdata/ip}

dump() {
    # dump STREAM FRAMES DESCRIPTION
    python3 tools/h264_model.py dump "testdata/ip/h264/$1.264" "$out" --frames "$2" --desc "$3"
}

# The list is also in tests/h264_model.rs (DUMPS); keep the two in step.
dump intra_32x32_qp26 1 "one IDR picture of 2x2 macroblocks, all Intra_4x4, QP 23"
dump intra_48x32_qp12 1 "one IDR picture of 3x2 macroblocks, Intra_4x4 and Intra_16x16, QP 12"
dump p_32x32_dump     3 "an IDR and two P pictures of 2x2 macroblocks, rotating: P_8x8 sub-partitions, two references"
dump craft_pcm        4 "2x2 macroblocks: an all-I_PCM IDR, I_PCM beside Intra_4x4, I_PCM inside two P pictures"
