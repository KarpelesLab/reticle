#!/usr/bin/env bash
#
# Makes the H.264 test streams that tools/h264_model.py is checked against,
# and the per-frame md5 of ffmpeg's decode of each (the reference).
#
# ffmpeg and libx264 are used here only as black-box programs: libx264
# writes the streams, ffmpeg's decoder writes the reference pictures. Their
# source is never read (see the clean-room rule in docs/h264-decoder.md).
#
# Usage: tools/h264_streams.sh [outdir]          the small committed corpus
#        tools/h264_streams.sh --hd [outdir]     only the 1280x720 stream
#
# Each stream NAME gets NAME.264 (Annex B byte stream) and NAME.md5 (one
# line per output frame: "<index> <md5 of the cropped I420 frame>"). The
# md5 is of ffmpeg's rawvideo yuv420p output, which is the cropped picture.
# Re-running regenerates identical files only with the same ffmpeg and
# libx264 builds; the committed files were made with ffmpeg 6.1.1.

set -euo pipefail

hd=0
if [ "${1:-}" = "--hd" ]; then hd=1; shift; fi
out=${1:-"$(dirname "$0")/../testdata/ip/h264"}
mkdir -p "$out"

# Common: one thread so the slice layout is deterministic; Baseline profile
# unless $profile says otherwise.
# The SEI that x264 writes (its version and option string) is stripped
# with filter_units: the decoder ignores SEI, and the corpus stays smaller.
base=(-hide_banner -loglevel error -y)
profile=baseline
x264() { echo -c:v libx264 -profile:v "$profile" -threads 1 -bsf:v filter_units=remove_types=6; }

ref() {
    # ref NAME: decode NAME.264 with ffmpeg and write NAME.md5
    local name=$1
    ffmpeg "${base[@]}" -i "$out/$name.264" -f rawvideo -pix_fmt yuv420p \
        -fps_mode passthrough "$out/$name.yuv"
    python3 "$(dirname "$0")/h264_md5.py" "$out/$name.264" "$out/$name.yuv" "$out/$name.md5"
    if [ -z "${KEEP_YUV:-}" ]; then rm -f "$out/$name.yuv"; fi
}

enc() {
    # enc NAME SOURCE FRAMES X264PARAMS [extra ffmpeg args...]
    local name=$1 src=$2 frames=$3 params=$4
    shift 4
    # shellcheck disable=SC2046
    ffmpeg "${base[@]}" -f lavfi -i "$src" -frames:v "$frames" $(x264) \
        -x264-params "$params" "$@" -f h264 "$out/$name.264"
    ref "$name"
}

craft() {
    # craft NAME: a stream written by tools/h264_craft.py, decoded by ffmpeg
    python3 "$(dirname "$0")/h264_craft.py" "$1" "$out/$1.264"
    ref "$1"
}

# Moving, detailed pictures. testsrc2 moves a few pixels a frame; a zooming
# mandelbrot is detailed but nearly still at first; noise gives big residuals.
ts() { echo "testsrc2=s=$1:r=30"; }
mand() { echo "mandelbrot=s=$1:r=30:start_scale=2:end_scale=0.3,format=yuv420p"; }
noisy() { echo "testsrc2=s=$1:r=30,noise=alls=60:allf=t"; }
# Rotation: every macroblock moves differently, by fractions of a sample,
# and the corners pull vectors outside the picture (reference clamping).
rot() { echo "testsrc2=s=$1:r=30,rotate=a=0.07*n:c=gray"; }
# The same, rendered at 4x and scaled down: finer detail, smaller motion.
rot4() { local w=${1%x*} h=${1#*x}; echo "testsrc2=s=$((4*w))x$((4*h)):r=30,scale=$w:$h,rotate=a=0.05*n:c=gray"; }

if [ "$hd" = 1 ]; then
    # Rotating, so every partition shape and fractional position occurs.
    enc hd_1280x720 "testsrc2=s=1280x720:r=30,rotate=a=0.01*n:c=gray" 3 \
        "keyint=60:crf=23:ref=2:partitions=all"
    profile=high
    enc hd_1280x720_high "testsrc2=s=1280x720:r=30,rotate=a=0.01*n:c=gray" 5 \
        "keyint=60:crf=23:ref=3:bframes=2:b-adapt=0:8x8dct=1:partitions=all"
    exit 0
fi

# Intra only: every I4x4 and I16x16 mode and every chroma mode, at a spread
# of QPs. QP 1 makes large coefficients (level_prefix escapes).
enc intra_32x32_qp26  "$(ts 32x32)"    3 "keyint=1:qp=26"
enc intra_48x32_qp1   "$(noisy 48x32)" 2 "keyint=1:qp=1"
enc intra_48x32_qp12  "$(noisy 48x32)" 2 "keyint=1:qp=12"
enc intra_64x48_qp38  "$(mand 64x48)"  2 "keyint=1:qp=38"
enc intra_64x48_qp51  "$(ts 64x48)"    2 "keyint=1:qp=51"
enc intra_80x48_i16   "$(mand 80x48)"  2 "keyint=1:qp=20:partitions=none"
# mb_qp_delta: adaptive quantisation moves QP from macroblock to macroblock.
enc intra_64x48_aq    "$(noisy 64x48)" 2 "keyint=1:crf=24:aq-mode=2:aq-strength=2"

# P slices: every partition shape and P_8x8 sub-partition (partitions=all
# enables p4x4), exhaustive subpel search so fractional vectors are common.
enc p_64x48_parts     "$(rot 64x48)"   6 "keyint=60:qp=24:partitions=all:subme=9:me=umh:ref=1"
enc p_48x32_parts     "$(rot4 48x32)"  6 "keyint=60:qp=20:partitions=all:subme=9:me=esa:ref=1"
enc p_80x48_skip      "$(ts 80x48)"    6 "keyint=60:qp=36:partitions=all:ref=1"
# A steady pan: P_Skip whose predicted vector is not zero, P_L0_16x16.
enc p_80x48_pan       "mandelbrot=s=160x96:r=30,format=yuv420p,scroll=h=0.006,scale=80:48" 6 \
    "keyint=60:qp=36:partitions=all:ref=1"
# Multiple reference frames (sliding window over 4 short-term frames).
enc p_64x48_ref4      "$(rot4 64x48)" 10 "keyint=60:qp=26:partitions=all:subme=9:ref=4:mixed-refs=1"
# mb_qp_delta in P slices.
enc p_64x48_aq        "$(noisy 64x48)" 5 "keyint=60:crf=26:aq-mode=2:aq-strength=2:partitions=all"
# Deblocking: off, and with offsets at both ends of the range.
enc p_48x32_nodeblock "$(rot 48x32)"   4 "keyint=60:qp=30:no-deblock=1:partitions=all"
enc p_48x32_dbk_m3p3  "$(rot 48x32)"   4 "keyint=60:qp=34:deblock=-3,3:partitions=all"
enc p_48x32_dbk_p6m6  "$(rot 48x32)"   4 "keyint=60:qp=40:deblock=6,-6:partitions=all"
# Several slices per picture: neighbour availability across a slice edge,
# and deblocking across it.
enc p_64x48_slices    "$(rot4 64x48)"  4 "keyint=60:qp=28:slices=3:partitions=all:ref=2"
# Cropping: sizes that are not a multiple of 16.
enc crop_30x18        "$(ts 30x18)"    4 "keyint=60:qp=24:partitions=all"
enc crop_50x38        "$(rot 50x38)"   4 "keyint=60:qp=28:partitions=all:ref=2"
# Intra macroblocks inside P slices (periodic intra refresh), with
# constrained_intra_pred: inter neighbours are unavailable to them.
enc p_64x48_cip       "$(rot 64x48)"   6 "keyint=4:intra-refresh=1:qp=30:partitions=all:constrained-intra=1"
# Small enough to dump in full for the hardware corpora (testdata/ip/h264_*).
enc p_32x32_dump      "$(rot 32x32)"   3 "keyint=60:qp=26:partitions=all:subme=9:ref=2"
# A second IDR inside the stream (keyint=3), with frame_num restarting.
enc idr_32x32_gop3    "$(rot 32x32)"   7 "keyint=3:min-keyint=1:scenecut=0:qp=28:partitions=all"

# What libx264 never writes, written by tools/h264_craft.py from the syntax
# of clause 7.3: I_PCM (alone, beside Intra_4x4, inside P slices), every
# memory_management_control_operation, long-term references, list
# modification of all three kinds, frame_num wrapping, a non-reference
# picture, picture order count types 0, 1 and 2, and
# disable_deblocking_filter_idc 2 with cropping on three sides. (A left
# crop is avoided on purpose: ffmpeg 6.1.1 does not apply one; it wrote
# 44-wide frames where the SPS said 42.)
craft craft_pcm
craft craft_mmco_poc0
craft craft_mmco_poc1
craft craft_mmco_poc2
craft craft_dbk2_crop

# Main and High: CABAC, B slices, the 8x8 transform, scaling matrices and
# weighted prediction. b-adapt=0 forces B pictures, which x264's adaptive
# decision otherwise drops for this much motion; Intra_8x8 needs pictures
# of 128x96 or so before x264 chooses it.
profile=main
enc cabac_i_qp1       "$(noisy 48x32)" 2 "keyint=1:qp=1"
enc cabac_p_64x48     "$(rot 64x48)"   6 "keyint=60:qp=26:bframes=0:partitions=all:ref=3"
enc cabac_aq_64x48    "$(noisy 64x48)" 5 "keyint=60:crf=26:aq-mode=2:aq-strength=2:bframes=0"
enc b_spatial_64x48   "$(rot 64x48)"   8 "keyint=60:qp=26:bframes=2:b-adapt=0:b-pyramid=none:direct=spatial:weightb=0:partitions=all"
enc b_temporal_64x48  "$(rot 64x48)"   8 "keyint=60:qp=26:bframes=2:b-adapt=0:b-pyramid=none:direct=temporal:weightb=0:partitions=all"
enc b_pyramid_64x48   "$(rot4 64x48)" 10 "keyint=60:qp=26:bframes=3:b-adapt=0:b-pyramid=normal:direct=auto:weightb=1:partitions=all:ref=3"
enc b_cavlc_64x48     "$(rot 64x48)"   8 "keyint=60:qp=26:bframes=2:b-adapt=0:cabac=0:direct=spatial:partitions=all"
enc weightp_64x48     "testsrc2=s=64x48:r=30,fade=t=in:st=0:d=0.3,rotate=a=0.03*n:c=gray" 8 \
    "keyint=60:qp=26:bframes=0:weightp=2"
profile=high
enc high_8x8_64x48    "$(rot4 64x48)"  6 "keyint=60:qp=24:bframes=2:b-adapt=0:8x8dct=1:partitions=all"
enc high_cqm_64x48    "$(rot4 64x48)"  6 "keyint=60:qp=24:bframes=2:b-adapt=0:8x8dct=1:cqm=jvt:partitions=all"
enc high_cavlc_64x48  "$(rot4 64x48)"  6 "keyint=60:qp=24:bframes=0:cabac=0:8x8dct=1:partitions=all"
enc i8_cabac_128x96   "$(mand 128x96)" 2 "keyint=1:qp=30:8x8dct=1"
enc i8_cavlc_128x96   "cellauto=s=128x96:r=30,format=yuv420p,boxblur=2" 2 "keyint=1:qp=24:8x8dct=1:cabac=0"
enc i8_cqm_128x96     "cellauto=s=128x96:r=30,format=yuv420p,boxblur=2" 2 "keyint=1:qp=24:8x8dct=1:cqm=jvt"
enc high_mix_128x96   "mandelbrot=s=128x96:r=30,format=yuv420p,rotate=a=0.02*n" 8 \
    "keyint=60:qp=26:8x8dct=1:bframes=2:b-adapt=0:partitions=all:ref=3"
