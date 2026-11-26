#!/usr/bin/env python3
"""Test streams and corpora for `ip/audio/aac_dec`.

The streams come from two encoders run as black boxes, ffmpeg's `aac` and
faac, and from a writer here that re-serialises a parsed stream with
changes: coding tools no encoder at hand emits (pulse data, 3-bit TNS
coefficients, correlated noise), and container features (ADTS CRC, several
raw_data_blocks in one ADTS frame, DSE and PCE elements). Every crafted
stream is still decoded by ffmpeg for the reference, so the reference never
comes from code written here.

Usage:
  aac_dec_streams.py streams <dir>          make every test stream in <dir>
  aac_dec_streams.py corpus <dir> <out.txt> write the corpus from them
  aac_dec_streams.py check <dir>            model vs ffmpeg for every stream
"""

import math
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import aac_dec_model as m  # noqa: E402

# ---------------------------------------------------------------------------
# Writing a bitstream
# ---------------------------------------------------------------------------


class BitWriter:
    def __init__(self):
        self.bits_ = []

    def bits(self, v, n):
        assert 0 <= v < (1 << n), (v, n)
        for i in range(n - 1, -1, -1):
            self.bits_.append((v >> i) & 1)

    def huff(self, book, idx):
        ln, cw = m.CODEBOOKS[book][idx]
        self.bits(cw, ln)

    def align(self):
        while len(self.bits_) % 8:
            self.bits_.append(0)

    def data(self):
        assert len(self.bits_) % 8 == 0
        out = bytearray()
        for i in range(0, len(self.bits_), 8):
            v = 0
            for b in self.bits_[i:i + 8]:
                v = (v << 1) | b
            out.append(v)
        return bytes(out)


def write_ics_info(bw, ics):
    bw.bits(0, 1)
    bw.bits(ics.window_sequence, 2)
    bw.bits(ics.window_shape, 1)
    if ics.window_sequence == m.EIGHT_SHORT:
        bw.bits(ics.max_sfb, 4)
        bw.bits(ics.grouping, 7)
    else:
        bw.bits(ics.max_sfb, 6)
        bw.bits(0, 1)


def tuple_index(book, vals):
    uns, dim, lav = m.CB_PARAMS[book]
    mod = lav + 1 if uns else 2 * lav + 1
    off = 0 if uns else lav
    idx = 0
    for v in vals:
        if uns:
            v = abs(v)
            if book == m.ESC_HCB and v > 15:
                v = 16
        idx = idx * mod + v + off
    return idx


def write_escape(bw, v):
    n = v.bit_length() - 5
    assert n >= 0
    bw.bits((1 << n) - 1 << 1, n + 1)  # n ones and a zero
    bw.bits(v - (1 << (n + 4)), n + 4)


def write_ics(bw, ics, common_window):
    bw.bits(ics.global_gain, 8)
    if not common_window:
        write_ics_info(bw, ics)
    esc = 7 if ics.window_sequence == m.EIGHT_SHORT else 31
    nb = 3 if ics.window_sequence == m.EIGHT_SHORT else 5
    for g in range(ics.num_groups):
        for cb, start, end in ics.sections[g]:
            bw.bits(cb, 4)
            ln = end - start
            while ln >= esc:
                bw.bits(esc, nb)
                ln -= esc
            bw.bits(ln, nb)
    sf, is_pos, nrg, first_noise = ics.global_gain, 0, ics.global_gain - 346, True
    for g in range(ics.num_groups):
        for sfb in range(ics.max_sfb):
            cb = ics.sfb_cb[g][sfb]
            v = ics.sf[g][sfb]
            if cb == m.ZERO_HCB:
                continue
            if cb in (m.INTENSITY_HCB, m.INTENSITY_HCB2):
                bw.huff(0, v - is_pos + 60)
                is_pos = v
            elif cb == m.NOISE_HCB:
                if first_noise:
                    first_noise = False
                    bw.bits(v - nrg, 9)
                else:
                    bw.huff(0, v - nrg + 60)
                nrg = v
            else:
                bw.huff(0, v - sf + 60)
                sf = v
    bw.bits(ics.pulse_present, 1)
    if ics.pulse_present:
        bw.bits(ics.pulse_num - 1, 2)
        bw.bits(ics.pulse_start, 6)
        for off, amp in ics.pulse:
            bw.bits(off, 5)
            bw.bits(amp, 4)
    bw.bits(ics.tns_present, 1)
    if ics.tns_present:
        short = ics.window_sequence == m.EIGHT_SHORT
        for w in range(ics.num_windows):
            coef_res, filters = ics.tns[w]
            bw.bits(len(filters), 1 if short else 2)
            if filters:
                bw.bits(coef_res, 1)
            for length, order, direction, compress, coefs in filters:
                bw.bits(length, 4 if short else 6)
                bw.bits(order, 3 if short else 5)
                if order:
                    bw.bits(direction, 1)
                    bw.bits(compress, 1)
                    for c in coefs:
                        bw.bits(c, coef_res + 3 - compress)
    bw.bits(0, 1)  # gain_control_data_present
    w0 = 0
    for g in range(ics.num_groups):
        glen = ics.group_len[g]
        order = []
        sfb_start = []
        for sfb in range(ics.max_sfb):
            sfb_start.append(len(order))
            width = ics.swb[sfb + 1] - ics.swb[sfb]
            for win in range(glen):
                base = (w0 + win) * 128 + ics.swb[sfb]
                order += [base + b for b in range(width)]
        sfb_start.append(len(order))
        for cb, start, end in ics.sections[g]:
            # (12 is reserved: only a stream made to be refused has it)
            if cb in (m.ZERO_HCB, m.NOISE_HCB, m.INTENSITY_HCB,
                      m.INTENSITY_HCB2, 12):
                continue
            uns, dim, lav = m.CB_PARAMS[cb]
            for k in range(sfb_start[start], sfb_start[end], dim):
                vals = [ics.qc[order[k + d]] for d in range(dim)]
                bw.huff(cb, tuple_index(cb, vals))
                if uns:
                    for v in vals:
                        if v:
                            bw.bits(1 if v < 0 else 0, 1)
                if cb == m.ESC_HCB:
                    for v in vals:
                        if abs(v) > 15:
                            write_escape(bw, abs(v))
        w0 += glen


def write_element(bw, el):
    if el.kind == m.ID_SCE:
        bw.bits(m.ID_SCE, 3)
        bw.bits(el.tag, 4)
        write_ics(bw, el.ics[0], False)
        return
    bw.bits(m.ID_CPE, 3)
    bw.bits(el.tag, 4)
    bw.bits(el.common_window, 1)
    a = el.ics[0]
    if el.common_window:
        write_ics_info(bw, a)
        bw.bits(el.ms_mask, 2)
        if el.ms_mask == 1:
            for g in range(a.num_groups):
                for sfb in range(a.max_sfb):
                    bw.bits(el.ms_used[g][sfb], 1)
    write_ics(bw, el.ics[0], el.common_window)
    write_ics(bw, el.ics[1], el.common_window)


def write_dse(bw, payload, align):
    bw.bits(m.ID_DSE, 3)
    bw.bits(0, 4)
    bw.bits(align, 1)
    cnt = len(payload)
    bw.bits(min(cnt, 255), 8)
    if cnt >= 255:
        bw.bits(cnt - 255, 8)
    if align:
        bw.align()
    for b in payload:
        bw.bits(b, 8)


def write_pce(bw):
    """A program_config_element describing one front channel pair, with a
    comment: what a stereo stream might carry beside its CPE."""
    bw.bits(m.ID_PCE, 3)
    bw.bits(0, 4)          # element_instance_tag
    bw.bits(1, 2)          # object_type (LC)
    bw.bits(3, 4)          # sampling_frequency_index
    bw.bits(1, 4)          # num_front_channel_elements
    bw.bits(0, 4)
    bw.bits(0, 4)
    bw.bits(0, 2)
    bw.bits(1, 3)          # one assoc data element
    bw.bits(0, 4)
    bw.bits(0, 1)          # no mono mixdown
    bw.bits(1, 1)          # stereo mixdown, element 2
    bw.bits(2, 4)
    bw.bits(1, 1)          # matrix mixdown
    bw.bits(1, 2)
    bw.bits(0, 1)
    bw.bits(1, 1)          # front: CPE, tag 0
    bw.bits(0, 4)
    bw.bits(5, 4)          # the assoc data element's tag
    bw.align()
    comment = b'reticle'
    bw.bits(len(comment), 8)
    for b in comment:
        bw.bits(b, 8)


def raw_block(elements, extra=()):
    """One raw_data_block: `extra` elements (callables on the writer) first,
    then the audio, then END and the alignment."""
    bw = BitWriter()
    for e in extra:
        e(bw)
    for el in elements:
        write_element(bw, el)
    bw.bits(m.ID_END, 3)
    bw.align()
    return bw.data()


def adts_frame(sfi, ch, blocks, crc=False):
    """An ADTS frame around one or more raw_data_blocks. The CRC words are
    written as zeros: this decoder does not check them, and neither does
    ffmpeg by default, so their value is not what is under test."""
    nrdb = len(blocks) - 1
    body = bytearray()
    if crc and nrdb:
        body += bytes(2 * nrdb)  # raw_data_block_position
    if crc:
        body += bytes(2)         # crc_check
    for i, blk in enumerate(blocks):
        body += blk
        if crc and nrdb:
            body += bytes(2)
    flen = 7 + len(body)
    bw = BitWriter()
    bw.bits(0xFFF, 12)
    bw.bits(0, 1)
    bw.bits(0, 2)
    bw.bits(0 if crc else 1, 1)
    bw.bits(1, 2)
    bw.bits(sfi, 4)
    bw.bits(0, 1)
    bw.bits(ch, 3)
    bw.bits(0, 4)
    bw.bits(flen, 13)
    bw.bits(0x7FF, 11)
    bw.bits(nrdb, 2)
    return bw.data() + bytes(body)


def rewrite(frames, **kw):
    """The frames written back out, one raw_data_block an ADTS frame."""
    out = bytearray()
    for sfi, ch, els in frames:
        out += adts_frame(sfi, ch, [raw_block(els)], **kw)
    return bytes(out)


# ---------------------------------------------------------------------------
# Changes no encoder at hand makes
# ---------------------------------------------------------------------------


def add_pulses(frames):
    """Moves part of some large coefficients of long windows into pulse
    data: a coefficient q coded as q - amp with a pulse of amp at its
    position decodes to q again (4.6.3.3). The spectrum is unchanged, so a
    decoder must produce what it produced for the original."""
    n = 0
    for sfi, ch, els in frames:
        for ics in els[0].ics:
            if ics.window_sequence == m.EIGHT_SHORT or ics.pulse_present:
                continue
            # candidate positions: in a coded band, |q| >= 3
            cands = []
            for sfb in range(ics.max_sfb):
                cb = ics.sfb_cb[0][sfb]
                if cb in (0, 13, 14, 15):
                    continue
                for k in range(ics.swb[sfb], ics.swb[sfb + 1]):
                    if abs(ics.qc[k]) >= 3:
                        cands.append((k, sfb))
            if not cands:
                continue
            start_sfb = cands[0][1]
            k0 = ics.swb[start_sfb]
            pulses = []
            last = k0
            for k, sfb in cands:
                if len(pulses) == 4:
                    break
                if k - last > 31:
                    continue
                q = ics.qc[k]
                amp = min(abs(q) - 1, 15)
                ics.qc[k] = q - amp if q > 0 else q + amp
                pulses.append((k - last, amp))
                last = k
            ics.pulse_present = 1
            ics.pulse_num = len(pulses)
            ics.pulse_start = start_sfb
            ics.pulse = pulses
            n += len(pulses)
    return n


def tns_res3(frames):
    """Rewrites every TNS filter with 4-bit coefficients as a 3-bit one
    (coef_res 0), halving each index: a different filter, which ffmpeg
    decodes as the reference."""
    n = 0
    for sfi, ch, els in frames:
        for ics in els[0].ics:
            if not ics.tns_present:
                continue
            new = []
            for coef_res, filters in ics.tns:
                if coef_res == 1:
                    nf = []
                    for length, order, direction, compress, coefs in filters:
                        bits = 4 - compress
                        out = []
                        for c in coefs:
                            t = c - (1 << bits) if c & (1 << (bits - 1)) else c
                            t = max(-4, min(3, t // 2))
                            out.append(t & 7)
                        nf.append((length, order, direction, 0, out))
                        n += 1
                    new.append((0, nf))
                else:
                    new.append((coef_res, filters))
            ics.tns = new
    return n


class Lcg:
    def __init__(self, seed):
        self.s = seed

    def below(self, n):
        self.s = (self.s * 6364136223846793005 + 1442695040888963407) % (1 << 64)
        return (self.s >> 33) % n


def tns_long(frames, seed=1):
    """Gives every long window filters that reach coded bands: two of them,
    of orders up to 12, with random coefficients in every resolution and
    compression, one upward and one downward. ffmpeg's encoder writes its
    long-window filters over the top bands only, past max_sfb, where they
    do nothing, so without this no long-window TNS would be exercised."""
    rnd = Lcg(seed)
    n = 0
    for sfi, ch, els in frames:
        for ics in els[0].ics:
            if ics.window_sequence == m.EIGHT_SHORT or ics.max_sfb < 4:
                continue
            top_reach = ics.num_swb - ics.max_sfb   # bands above max_sfb
            filters = []
            for f in range(2):
                order = 12 - rnd.below(5) if f == 0 else 1 + rnd.below(12)
                length = (top_reach + ics.max_sfb // 2) if f == 0 \
                    else ics.max_sfb // 2 + rnd.below(4)
                compress = rnd.below(2)
                # small indexes: reflection coefficients up to about 0.4,
                # a filter whose gain stays well inside 16 bits
                coefs = [rnd.below(3) - 1 for _ in range(order)]
                filters.append((min(length, 63), order, f, compress, coefs))
            # one resolution per window, as the syntax has it
            res = rnd.below(2)
            fixed = []
            for length, order, direction, compress, coefs in filters:
                bits = res + 3 - compress
                fixed.append((length, order, direction, compress,
                              [c & ((1 << bits) - 1) for c in coefs]))
            ics.tns = [(res, fixed)]
            ics.tns_present = 1
            n += 1
    return n


def big_escapes(frames):
    """Puts escape values up to the largest the standard allows (8191,
    4.6.1.3) into one ESC-coded band of each long window, with the band's
    scalefactor lowered so that the result stays inside sixteen bits:
    exercises the escapes of eight to twelve bits and the interpolation of
    |q|^(4/3) above 1024. Returns how many values were placed."""
    big = [8191, -8191, 1025, -1031, 2047, 4100, -6001, 1500]
    n = 0
    for sfi, ch, els in frames:
        for ics in els[0].ics:
            if ics.window_sequence == m.EIGHT_SHORT:
                continue
            for sfb in range(ics.max_sfb):
                if ics.sfb_cb[0][sfb] != m.ESC_HCB:
                    continue
                # the scalefactor that puts 8191^(4/3) near 2^21
                want = 100 + int(round(4 * math.log2(2 ** 21 / 8191 ** (4 / 3))))
                prev = [ics.sf[0][b] for b in range(sfb) if
                        ics.sfb_cb[0][b] not in (0, 13, 14, 15)]
                nxt = [ics.sf[0][b] for b in range(sfb + 1, ics.max_sfb) if
                       ics.sfb_cb[0][b] not in (0, 13, 14, 15)]
                before = prev[-1] if prev else ics.global_gain
                after = nxt[0] if nxt else want
                want = max(want, before - 60, after - 60)
                want = min(want, before + 60, after + 60)
                if not (before - 60 <= want <= before + 60 and
                        after - 60 <= want <= after + 60):
                    continue
                ics.sf[0][sfb] = want
                lo = ics.swb[sfb]
                for i, v in enumerate(big[:ics.swb[sfb + 1] - lo]):
                    ics.qc[lo + i] = v
                    n += 1
                break
    return n


def split_pair(left, right):
    """A stereo stream whose CPE has no common window, from two mono
    streams at one rate: each channel keeps its own ics_info, so the two
    switch window sequences independently (one long while the other is
    short). No encoder at hand writes such a pair."""
    out = []
    for (sfi, _, els_l), (_, _, els_r) in zip(left, right):
        el = m.Element()
        el.kind = m.ID_CPE
        el.tag = 0
        el.common_window = 0
        el.ms_mask = 0
        el.ms_used = None
        el.ics = [els_l[0].ics[0], els_r[0].ics[0]]
        out.append((sfi, 2, [el]))
    return out


def correlate_noise(frames):
    """Sets ms_used on every band where both channels of a common-window
    CPE are noise: the right channel then replays the left one's noise
    (4.6.13.3). Returns how many bands that is."""
    n = 0
    for sfi, ch, els in frames:
        el = els[0]
        if el.kind != m.ID_CPE or not el.common_window:
            continue
        a, b = el.ics
        hit = []
        for g in range(a.num_groups):
            for sfb in range(a.max_sfb):
                if a.sfb_cb[g][sfb] == m.NOISE_HCB and \
                        b.sfb_cb[g][sfb] == m.NOISE_HCB:
                    hit.append((g, sfb))
        if not hit:
            continue
        if el.ms_mask != 1:
            el.ms_used = [[1 if el.ms_mask == 2 else 0
                           for _ in range(a.max_sfb)]
                          for _ in range(a.num_groups)]
            el.ms_mask = 1
        for g, sfb in hit:
            el.ms_used[g][sfb] = 1
        n += len(hit)
    return n


def multi_rdb(frames, per=3, crc=False):
    """Several raw_data_blocks in each ADTS frame (number_of_raw_data_blocks
    _in_frame > 0), with or without the CRC words that come with them."""
    out = bytearray()
    for i in range(0, len(frames), per):
        group = frames[i:i + per]
        sfi, ch = group[0][0], group[0][1]
        out += adts_frame(sfi, ch, [raw_block(els) for _, _, els in group],
                          crc=crc)
    return bytes(out)


def with_extras(frames):
    """A DSE (aligned and not), a PCE and a FIL in front of the audio of
    each raw_data_block: elements a decoder must step over."""
    out = bytearray()
    for i, (sfi, ch, els) in enumerate(frames):
        payload = bytes((i * 7 + k) & 0xFF for k in range(5 + i % 3))

        def fil(bw, n=3 + i % 4):
            bw.bits(m.ID_FIL, 3)
            bw.bits(n, 4)
            bw.bits(0, 4)   # EXT_FILL with a fill_nibble... any bits do
            for _ in range(n - 1):
                bw.bits(0xA5, 8)
            bw.bits(0xA, 4)
        extra = [lambda bw, p=payload, a=i % 2: write_dse(bw, p, a), fil]
        if i % 3 == 0:
            extra.insert(0, write_pce)
        out += adts_frame(sfi, ch, [raw_block(els, extra)])
    return bytes(out)


# ---------------------------------------------------------------------------
# The streams
# ---------------------------------------------------------------------------

# name, lavfi source, extra ffmpeg arguments
FFMPEG_STREAMS = [
    ('sine_m48', 'sine=frequency=1000:sample_rate=48000:duration=0.2',
     ['-ac', '1', '-b:a', '96k', '-aac_pns', '0']),
    ('tones_s44', 'aevalsrc=0.5*sin(2*PI*440*t)|0.4*sin(2*PI*660*t):s=44100:d=0.25',
     ['-b:a', '128k', '-aac_pns', '0']),
    ('corr_s48', 'aevalsrc=0.6*sin(2*PI*300*t)+0.2*sin(2*PI*5000*t)|'
     '0.6*sin(2*PI*300*t)+0.18*sin(2*PI*5000*t+0.1):s=48000:d=0.25',
     ['-b:a', '96k', '-aac_pns', '0']),
    ('noise_s48_lo', 'anoisesrc=color=pink:amplitude=0.4:sample_rate=48000:'
     'duration=0.3:seed=7', ['-ac', '2', '-b:a', '32k']),
    ('noise_s48_is', 'aevalsrc=0.3*sin(2*PI*200*t)+0.3*(random(0)-0.5)|'
     '0.25*sin(2*PI*200*t)+0.3*(random(0)-0.5)*0.9:s=48000:d=0.3',
     ['-b:a', '40k', '-aac_is', '1', '-aac_pns', '1']),
    ('clicks_m44', 'aevalsrc=if(lt(mod(t\\,0.05)\\,0.002)\\,0.8*sin(2*PI*3000*t)'
     '\\,0.01*sin(2*PI*200*t)):s=44100:d=0.3', ['-ac', '1', '-b:a', '96k',
                                                  '-aac_pns', '0']),
    ('sweep_s32', 'aevalsrc=0.7*sin(2*PI*(100+20000*t)*t)|'
     '0.5*sin(2*PI*(100+15000*t)*t):s=32000:d=0.3', ['-b:a', '96k',
                                                    '-aac_pns', '0']),
    ('sine_m24', 'sine=frequency=700:sample_rate=24000:duration=0.3',
     ['-ac', '1', '-b:a', '48k', '-aac_pns', '0']),
    ('sine_m22', 'sine=frequency=900:sample_rate=22050:duration=0.3',
     ['-ac', '1', '-b:a', '48k', '-aac_pns', '0']),
    ('tones_s16', 'aevalsrc=0.5*sin(2*PI*440*t)|0.5*sin(2*PI*1200*t):s=16000:d=0.3',
     ['-b:a', '48k', '-aac_pns', '0']),
    ('sine_m12', 'sine=frequency=500:sample_rate=12000:duration=0.3',
     ['-ac', '1', '-b:a', '24k', '-aac_pns', '0']),
    ('sine_m11', 'sine=frequency=500:sample_rate=11025:duration=0.3',
     ['-ac', '1', '-b:a', '24k', '-aac_pns', '0']),
    ('sine_m8', 'sine=frequency=400:sample_rate=8000:duration=0.4',
     ['-ac', '1', '-b:a', '16k', '-aac_pns', '0']),
    ('loud_s48', 'aevalsrc=0.99*sin(2*PI*100*t)|-0.99*sin(2*PI*100*t):s=48000:d=0.2',
     ['-b:a', '192k', '-aac_pns', '0']),
    ('square_m48', 'aevalsrc=0.9*(2*gte(mod(t*220\\,1)\\,0.5)-1):s=48000:d=0.2',
     ['-ac', '1', '-b:a', '128k', '-aac_pns', '0']),
]

# name, lavfi source of the WAV, faac arguments
FAAC_STREAMS = [
    ('faac_s44', 'aevalsrc=0.5*sin(2*PI*440*t)+0.2*sin(2*PI*3500*t)|'
     '0.5*sin(2*PI*440*t)+0.25*sin(2*PI*2500*t):s=44100:d=0.3', ['-b', '96']),
    ('faac_clk48', 'aevalsrc=if(lt(mod(t\\,0.07)\\,0.003)\\,0.9*sin(2*PI*2000*t)'
     '\\,0.02*sin(2*PI*300*t)):s=48000:d=0.3', ['-b', '64']),
    ('faac_noise48', 'anoisesrc=color=white:amplitude=0.3:sample_rate=48000:'
     'duration=0.3:seed=3', ['-b', '48']),
]


def run(*args):
    subprocess.run(list(args), check=True)


def channels(path):
    out = subprocess.run(['ffprobe', '-v', 'error', '-show_entries',
                          'stream=channels', '-of', 'csv=p=0', path],
                         check=True, capture_output=True, text=True)
    return int(out.stdout.strip())


SAME_AS = {'craft_multi_s16': 'tones_s16', 'craft_multi_crc_m22': 'sine_m22'}


def make_streams(d):
    os.makedirs(d, exist_ok=True)
    for name, src, args in FFMPEG_STREAMS:
        run('ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', src, '-c:a',
            'aac', *args, os.path.join(d, name + '.aac'))
    for name, src, args in FAAC_STREAMS:
        wav = os.path.join(d, name + '.wav')
        ch = ['-ac', '2'] if 'anoisesrc' in src else []
        run('ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', src, *ch,
            '-c:a', 'pcm_s16le', wav)
        run('faac', '-v0', *args, '-o', os.path.join(d, name + '.aac'), wav)
        os.remove(wav)

    def load(name):
        return m.frames_of(os.path.join(d, name + '.aac'))

    def save(name, data):
        with open(os.path.join(d, name + '.aac'), 'wb') as f:
            f.write(data)

    fr = load('tones_s44')
    add_pulses(fr)
    save('craft_pulse_s44', rewrite(fr))
    fr = load('sine_m48')
    add_pulses(fr)
    save('craft_pulse_m48', rewrite(fr, crc=True))
    fr = load('noise_s48_lo')
    tns_long(fr)
    save('craft_tns_s48', rewrite(fr))
    fr = load('sine_m24')
    tns_long(fr, seed=5)
    save('craft_tns_m24', rewrite(fr))
    fr = load('corr_s48')
    tns_res3(fr)
    save('craft_tns3_s48', rewrite(fr))
    save('craft_split_s48', rewrite(split_pair(load('sine_m48'),
                                               load('square_m48'))))
    fr = load('sine_m48')
    big_escapes(fr)
    save('craft_esc_m48', rewrite(fr))
    fr = load('noise_s48_is')
    correlate_noise(fr)
    save('craft_corr_noise_s48', rewrite(fr))
    save('craft_multi_s16', multi_rdb(load('tones_s16'), per=3))
    save('craft_multi_crc_m22', multi_rdb(load('sine_m22'), per=2, crc=True))
    save('craft_extras_s44', with_extras(load('tones_s44')))
    for f in sorted(os.listdir(d)):
        if f.endswith('.aac') and f[:-4] not in SAME_AS:
            path = os.path.join(d, f)
            run('ffmpeg', '-v', 'error', '-y', '-i', path, '-f', 's16le',
                '-ac', str(channels(path)), path[:-4] + '.s16')
    # ffmpeg 6.1 refuses an ADTS frame of more than one raw_data_block
    # ("not implemented"), so those streams take the PCM ffmpeg made of
    # the same blocks one to a frame.
    for name, orig in SAME_AS.items():
        with open(os.path.join(d, orig + '.s16'), 'rb') as f:
            pcm = f.read()
        with open(os.path.join(d, name + '.s16'), 'wb') as f:
            f.write(pcm)


def check_streams(d):
    for f in sorted(os.listdir(d)):
        if not f.endswith('.aac'):
            continue
        path = os.path.join(d, f)
        cov = m.Coverage()
        m.frames_of(path, cov)
        ref = m.read_s16(path[:-4] + '.s16')
        res = m.compare_frames(path, ref)
        used = ' '.join(k for k, v in cov.c.items() if v and k not in
                        ('frames',) and not k.startswith('cb'))
        print('%-22s %s\n    %s' % (f[:-4], m.describe(res), used))


# ---------------------------------------------------------------------------
# The corpus
# ---------------------------------------------------------------------------

# name, raw_data_blocks to keep, the frames whose ffmpeg PCM goes in too
CORPUS = [
    ('sine_m48', 5, (1, 2, 3)),
    ('corr_s48', 3, (1,)),
    ('tones_s44', 3, ()),
    ('loud_s48', 3, ()),
    ('noise_s48_is', 3, ()),
    ('craft_corr_noise_s48', 3, ()),
    ('craft_tns_m24', 3, ()),
    ('craft_esc_m48', 3, ()),
    ('craft_split_s48', 4, ()),
    ('craft_tns3_s48', 2, ()),
    ('craft_pulse_s44', 2, ()),
    ('craft_extras_s44', 2, ()),
    ('craft_multi_crc_m22', 4, ()),
    ('sweep_s32', 3, ()),
    ('tones_s16', 2, ()),
    ('sine_m11', 2, ()),
    ('sine_m8', 2, ()),
    ('faac_s44', 3, ()),
    ('faac_clk48', 3, ()),
]


def fnv64(pcm):
    """FNV-1a, 64 bits, over the samples as little-endian 16-bit words."""
    h = 0xCBF29CE484222325
    for v in pcm:
        for b in (v & 0xFF, (v >> 8) & 0xFF):
            h ^= b
            h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def adts_prefix(data, blocks):
    """The bytes of the ADTS frames holding the first `blocks`
    raw_data_blocks."""
    pos = n = 0
    while n < blocks:
        flen = ((data[pos + 3] & 3) << 11) | (data[pos + 4] << 3) | \
            (data[pos + 5] >> 5)
        n += (data[pos + 6] & 3) + 1
        pos += flen
    return data[:pos]


def corpus_record(d, name, keep, with_pcm):
    path = os.path.join(d, name + '.aac')
    data = open(path, 'rb').read()
    cov = m.Coverage()
    frames = m.frames_of(path, cov)
    ref = m.read_s16(os.path.join(d, name + '.s16'))
    res = m.compare_frames(path, ref)
    off = res['offset']
    sfi, ch = frames[0][0], frames[0][1]
    prefix = adts_prefix(data, keep)
    cov = m.Coverage()
    kept = list(m.parse_adts(prefix, cov))
    dec = m.FixedDecoder()
    lines = ['stream %s' % name]
    tools = [k for k, v in cov.c.items() if v and k != 'frames']
    lines.append('info sfi %d ch %d frames %d offset %d' %
                 (sfi, ch, len(kept), off))
    lines.append('tools ' + ' '.join(tools))
    for i in range(0, len(prefix), 48):
        lines.append('adts ' + prefix[i:i + 48].hex())
    pcm_lines = []
    for f, fr in enumerate(kept):
        outs = dec.decode(*fr)
        pcm = [o[n] for n in range(1024) for o in outs]
        lines.append('model %d %016x' % (f, fnv64(pcm)))
        rf = f - off   # the ffmpeg frame that lines up
        if rf < 0 or (rf + 1) * 1024 * ch > len(ref):
            lines.append('ffmpeg %d none' % f)
            continue
        rpcm = ref[rf * 1024 * ch:(rf + 1) * 1024 * ch]
        noisy = m.has_noise(fr[2]) or (f > 0 and m.has_noise(kept[f - 1][2]))
        e2 = sum((a - b) ** 2 for a, b in zip(pcm, rpcm))
        s2 = sum(b * b for b in rpcm)
        mx = max(abs(a - b) for a, b in zip(pcm, rpcm))
        mis = sum(1 for a, b in zip(pcm, rpcm) if a != b)
        kind = 'first' if f == 0 else ('noise' if noisy else 'exact')
        lines.append('ffmpeg %d %s max %d differ %d err2 %d sig2 %d ours2 %d' %
                     (f, kind, mx, mis, e2, s2, sum(a * a for a in pcm)))
        if f in with_pcm:
            for i in range(0, len(rpcm), 32):
                pcm_lines.append('pcm %d ' % f + ''.join(
                    '%04x' % (v & 0xFFFF) for v in rpcm[i:i + 32]))
    lines += pcm_lines
    lines.append('end')
    return lines


CORPUS_HEADER = '''\
# aac_dec corpus: ADTS streams, the bit-accurate model's PCM for each frame,
# and what ffmpeg's decoder made of the same frames.
#
# Written by `tools/aac_dec_streams.py corpus`; the streams by its `streams`
# command, from ffmpeg's and faac's encoders run as black boxes and from
# re-serialisations of their output (the `craft_` streams, see that file).
# ffmpeg 6.1.1 and faac 1.30 made these.
#
#   stream <name>
#   info sfi <i> ch <n> frames <n> offset <frames ffmpeg dropped first>
#   tools <the coding tools and elements the kept frames' stream uses>
#   adts <hex>            the stream's first frames, every byte
#   model <f> <hash>      FNV-1a 64 of the model's frame f, interleaved s16 LE
#   ffmpeg <f> <kind> max <m> differ <n> err2 <e> sig2 <s> ours2 <o>
#                         the model against ffmpeg on frame f: `exact`
#                         frames are compared sample by sample; `noise`
#                         frames hold PNS (or follow one), whose noise is
#                         random, so only by energy; `first` is the frame
#                         ffmpeg starts from a sine left half
#   pcm <f> <hex>         ffmpeg's own samples of frame f, for the streams
#                         compared with ffmpeg directly
#   end
'''


def junk(n, seed):
    """n bytes that are not 0xFF, so no syncword hides in them."""
    rnd = Lcg(seed)
    return bytes(rnd.below(255) for _ in range(n))


def error_streams(d):
    """Streams the decoder must refuse in part, as (name, bytes, the frame
    list a decoder that skips the refused frames decodes, the error codes
    it reports in order). The frames come from sine_m48."""
    out = []

    def frames():
        return m.frames_of(os.path.join(d, 'sine_m48.aac'))

    # Bytes before, and between, frames: a lost syncword (code 1), once
    # for each stretch.
    fr = frames()
    data = junk(37, 3) + rewrite(fr[0:2]) + junk(11, 4) + rewrite(fr[2:5])
    out.append(('err_sync_m48', data, fr[0:5], [1, 1]))

    # A CCE (code 3), a reserved codebook (code 5), and a frame whose ADTS
    # length is shorter than its data (code 9): each frame is skipped and
    # the next one decoded, from the state the last good frame left.
    fr = frames()
    blocks = []
    sfi, ch = fr[0][0], fr[0][1]
    blocks.append(adts_frame(sfi, ch, [raw_block(fr[0][2])]))
    blocks.append(adts_frame(sfi, ch, [raw_block(fr[1][2], [
        lambda bw: (bw.bits(m.ID_CCE, 3), bw.bits(0, 16))])]))
    blocks.append(adts_frame(sfi, ch, [raw_block(fr[2][2])]))
    bad = m.frames_of(os.path.join(d, 'sine_m48.aac'))[3]
    ics = bad[2][0].ics[0]
    cb, start, end = ics.sections[0][0]
    ics.sections[0][0] = (12, start, end)
    blocks.append(adts_frame(sfi, ch, [raw_block(bad[2])]))
    blocks.append(adts_frame(sfi, ch, [raw_block(fr[4][2])]))
    short = bytearray(adts_frame(sfi, ch, [raw_block(fr[5][2])]))
    flen = len(short) - 40
    short[3] = (short[3] & 0xFC) | (flen >> 11)
    short[4] = (flen >> 3) & 0xFF
    short[5] = ((flen & 7) << 5) | (short[5] & 0x1F)
    blocks.append(bytes(short[:flen]))
    blocks.append(adts_frame(sfi, ch, [raw_block(fr[6][2])]))
    out.append(('err_frames_m48', b''.join(blocks),
                [fr[0], fr[2], fr[4], fr[6]], [3, 5, 9]))
    return out


def error_record(name, data, good, errors):
    dec = m.FixedDecoder()
    sfi, ch = good[0][0], good[0][1]
    lines = ['stream %s' % name,
             'info sfi %d ch %d frames %d offset 0' % (sfi, ch, len(good)),
             'errors ' + ' '.join(str(e) for e in errors)]
    for i in range(0, len(data), 48):
        lines.append('adts ' + data[i:i + 48].hex())
    for f, fr in enumerate(good):
        outs = dec.decode(*fr)
        pcm = [o[n] for n in range(1024) for o in outs]
        lines.append('model %d %016x' % (f, fnv64(pcm)))
    lines.append('end')
    return lines


def write_corpus(d, out):
    lines = [CORPUS_HEADER]
    for name, keep, with_pcm in CORPUS:
        lines += corpus_record(d, name, keep, with_pcm)
        lines.append('')
    for name, data, good, errors in error_streams(d):
        lines += error_record(name, data, good, errors)
        lines.append('')
    with open(out, 'w') as f:
        f.write('\n'.join(lines))


if __name__ == '__main__':
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    if sys.argv[1] == 'streams':
        make_streams(sys.argv[2])
    elif sys.argv[1] == 'check':
        check_streams(sys.argv[2])
    elif sys.argv[1] == 'corpus':
        write_corpus(sys.argv[2], sys.argv[3])
    else:
        raise SystemExit(__doc__)
