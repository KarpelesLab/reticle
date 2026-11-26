#!/usr/bin/env python3
"""A clean-room model of `ip/audio/aac_dec`, the AAC-LC decoder.

Written from ISO/IEC 14496-3 subpart 4 (and subpart 1 for ADTS) and from
nothing else: no other decoder's source was read. It has two halves.

* `parse_*`: the bitstream syntax, shared by both decoders below. It also
  counts which coding tools a stream exercises (`Coverage`), which is how
  the package README knows what its test streams cover.
* `FloatDecoder`: the standard's arithmetic in double precision, straight
  from the formulas (a direct O(N^2) IMDCT, the direct-form TNS filter).
  It exists to check that this reading of the standard agrees with an
  independent decoder (ffmpeg, run as a black box) before any fixed-point
  choice is made.
* `FixedDecoder`: the bit-accurate model of the RTL. Every rounding,
  every saturation and every table entry is the one the hardware makes,
  so the RTL must reproduce its PCM output exactly.

Usage:
  aac_dec_model.py stats  <file.aac> [--raw SFI CH]
  aac_dec_model.py decode <file.aac> <out.s16> [--float] [--raw SFI CH]
  aac_dec_model.py compare <file.aac> <ffmpeg.s16> [--float]
  aac_dec_model.py tables <rtl-dir>        (writes the generated ROMs)
"""

import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

# ---------------------------------------------------------------------------
# Tables quoted from the standard
# ---------------------------------------------------------------------------

# Table 1.18: sampling frequency index -> Hz.
SAMPLE_RATES = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050,
                16000, 12000, 11025, 8000, 7350]

# Tables 4.129-4.141: swb_offset_long_window / swb_offset_short_window, with
# the closing 1024 / 128. Only the rates up to 48 kHz this decoder accepts.
SWB_LONG_48 = [0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 48, 56, 64, 72, 80,
               88, 96, 108, 120, 132, 144, 160, 176, 196, 216, 240, 264, 292,
               320, 352, 384, 416, 448, 480, 512, 544, 576, 608, 640, 672, 704,
               736, 768, 800, 832, 864, 896, 928, 1024]
SWB_LONG_32 = SWB_LONG_48[:-1] + [960, 992, 1024]
SWB_LONG_24 = [0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 52, 60, 68, 76,
               84, 92, 100, 108, 116, 124, 136, 148, 160, 172, 188, 204, 220,
               240, 260, 284, 308, 336, 364, 396, 432, 468, 508, 552, 600, 652,
               704, 768, 832, 896, 960, 1024]
SWB_LONG_16 = [0, 8, 16, 24, 32, 40, 48, 56, 64, 72, 80, 88, 100, 112, 124,
               136, 148, 160, 172, 184, 196, 212, 228, 244, 260, 280, 300, 320,
               344, 368, 396, 424, 456, 492, 532, 572, 616, 664, 716, 772, 832,
               896, 960, 1024]
SWB_LONG_8 = [0, 12, 24, 36, 48, 60, 72, 84, 96, 108, 120, 132, 144, 156, 172,
              188, 204, 220, 236, 252, 268, 288, 308, 328, 348, 372, 396, 420,
              448, 476, 508, 544, 580, 620, 664, 712, 764, 820, 880, 944, 1024]
SWB_SHORT_48 = [0, 4, 8, 12, 16, 20, 28, 36, 44, 56, 68, 80, 96, 112, 128]
SWB_SHORT_24 = [0, 4, 8, 12, 16, 20, 24, 28, 36, 44, 52, 64, 76, 92, 108, 128]
SWB_SHORT_16 = [0, 4, 8, 12, 16, 20, 24, 28, 32, 40, 48, 60, 72, 88, 108, 128]
SWB_SHORT_8 = [0, 4, 8, 12, 16, 20, 24, 28, 36, 44, 52, 60, 72, 88, 108, 128]

# Index 3..12 (48 kHz down to 7350 Hz). 7350 Hz uses the 8 kHz tables
# (Table 4.82). Each: (long offsets, short offsets, TNS_MAX_BANDS long,
# TNS_MAX_BANDS short) -- Table 4.157, "without PQF filterbank".
RATE_TABLES = {
    3: (SWB_LONG_48, SWB_SHORT_48, 40, 14),
    4: (SWB_LONG_48, SWB_SHORT_48, 42, 14),
    5: (SWB_LONG_32, SWB_SHORT_48, 51, 14),
    6: (SWB_LONG_24, SWB_SHORT_24, 46, 14),
    7: (SWB_LONG_24, SWB_SHORT_24, 46, 14),
    8: (SWB_LONG_16, SWB_SHORT_16, 42, 14),
    9: (SWB_LONG_16, SWB_SHORT_16, 42, 14),
    10: (SWB_LONG_16, SWB_SHORT_16, 42, 14),
    11: (SWB_LONG_8, SWB_SHORT_8, 39, 14),
    12: (SWB_LONG_8, SWB_SHORT_8, 39, 14),
}

ONLY_LONG, LONG_START, EIGHT_SHORT, LONG_STOP = 0, 1, 2, 3
ZERO_HCB, ESC_HCB, NOISE_HCB, INTENSITY_HCB2, INTENSITY_HCB = 0, 11, 13, 14, 15
ID_SCE, ID_CPE, ID_CCE, ID_LFE, ID_DSE, ID_PCE, ID_FIL, ID_END = range(8)

# Table 4.151: (unsigned, dimension, lav) for codebooks 1..11.
CB_PARAMS = {1: (0, 4, 1), 2: (0, 4, 1), 3: (1, 4, 2), 4: (1, 4, 2),
             5: (0, 2, 4), 6: (0, 2, 4), 7: (1, 2, 7), 8: (1, 2, 7),
             9: (1, 2, 12), 10: (1, 2, 12), 11: (1, 2, 16)}

TNS_MAX_ORDER_LONG, TNS_MAX_ORDER_SHORT = 12, 7


def check_rate_tables():
    """The swb tables were typed from the standard; hold them to what the
    standard says of them: the band counts of Tables 4.129-4.141, strictly
    increasing, and a multiple of four everywhere (4.6.3.1)."""
    counts = {id(SWB_LONG_48): 49, id(SWB_LONG_32): 51, id(SWB_LONG_24): 47,
              id(SWB_LONG_16): 43, id(SWB_LONG_8): 40, id(SWB_SHORT_48): 14,
              id(SWB_SHORT_24): 15, id(SWB_SHORT_16): 15, id(SWB_SHORT_8): 15}
    for t in (SWB_LONG_48, SWB_LONG_32, SWB_LONG_24, SWB_LONG_16, SWB_LONG_8,
              SWB_SHORT_48, SWB_SHORT_24, SWB_SHORT_16, SWB_SHORT_8):
        assert len(t) == counts[id(t)] + 1, t
        assert t[0] == 0 and t[-1] in (1024, 128)
        assert all(a < b for a, b in zip(t, t[1:]))
        assert all(v % 4 == 0 for v in t)


def load_codebooks():
    """Tables 4.A.1-4.A.12 as {book: {index: (length, codeword)}}; book 0 is
    the scalefactor codebook."""
    books = {}
    with open(os.path.join(HERE, 'aac_dec_codebooks.txt')) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            b, idx, ln, cw = line.split()
            books.setdefault(int(b), {})[int(idx)] = (int(ln), int(cw, 16))
    return books


CODEBOOKS = load_codebooks()


def decode_tables():
    """{book: {(length, codeword): index}} for the reference parser."""
    out = {}
    for b, entries in CODEBOOKS.items():
        out[b] = {v: k for k, v in entries.items()}
    return out


DECODE = decode_tables()

# ---------------------------------------------------------------------------
# Bitstream
# ---------------------------------------------------------------------------


class AacError(Exception):
    pass


class BitReader:
    def __init__(self, data, pos=0):
        self.data = data
        self.pos = pos  # in bits

    def bits(self, n):
        v = 0
        for _ in range(n):
            byte = self.pos >> 3
            if byte >= len(self.data):
                raise AacError('read past the end of the data')
            v = (v << 1) | ((self.data[byte] >> (7 - (self.pos & 7))) & 1)
            self.pos += 1
        return v

    def align(self):
        self.pos = (self.pos + 7) & ~7

    def huff(self, book):
        table = DECODE[book]
        v = 0
        for ln in range(1, 20):
            v = (v << 1) | self.bits(1)
            hit = table.get((ln, v))
            if hit is not None:
                return hit
        raise AacError('no codeword of book %d' % book)


class Coverage:
    """Which coding tools a stream exercised, counted."""
    KEYS = ['frames', 'sce', 'cpe', 'fil', 'dse', 'pce', 'common_window',
            'only_long', 'long_start', 'eight_short', 'long_stop',
            'shape_sine', 'shape_kbd', 'grouped_short', 'ms_mask_1',
            'ms_mask_2', 'ms_bands', 'intensity_bands', 'intensity_hcb2',
            'noise_bands', 'noise_correlated', 'pulse', 'tns', 'tns_short',
            'tns_downward', 'tns_compress', 'tns_res3', 'esc_values',
            'independent_windows',
            'esc_over_1023', 'adts_crc', 'multi_rdb'] + \
        ['cb%d' % i for i in range(1, 12)]

    def __init__(self):
        self.c = {k: 0 for k in self.KEYS}

    def add(self, k, n=1):
        self.c[k] += n

    def __str__(self):
        return ' '.join('%s=%d' % (k, v) for k, v in self.c.items())


class Ics:
    pass


def window_info(ics, sfi):
    long_t, short_t, tmb_l, tmb_s = RATE_TABLES[sfi]
    if ics.window_sequence == EIGHT_SHORT:
        ics.num_windows = 8
        ics.swb = short_t
        ics.num_swb = len(short_t) - 1
        ics.tns_max_bands = tmb_s
        lens = [1]
        for i in range(7):
            if (ics.grouping >> (6 - i)) & 1:
                lens[-1] += 1
            else:
                lens.append(1)
        ics.group_len = lens
    else:
        ics.num_windows = 1
        ics.swb = long_t
        ics.num_swb = len(long_t) - 1
        ics.tns_max_bands = tmb_l
        ics.group_len = [1]
    ics.num_groups = len(ics.group_len)
    if ics.max_sfb > ics.num_swb:
        raise AacError('max_sfb %d > num_swb %d' % (ics.max_sfb, ics.num_swb))


def parse_ics_info(br, ics, sfi):
    if br.bits(1):
        raise AacError('ics_reserved_bit set')
    ics.window_sequence = br.bits(2)
    ics.window_shape = br.bits(1)
    if ics.window_sequence == EIGHT_SHORT:
        ics.max_sfb = br.bits(4)
        ics.grouping = br.bits(7)
    else:
        ics.max_sfb = br.bits(6)
        ics.grouping = 0
        if br.bits(1):
            raise AacError('predictor_data_present in AAC LC')
    window_info(ics, sfi)


def parse_section_data(br, ics):
    esc = 7 if ics.window_sequence == EIGHT_SHORT else 31
    nb = 3 if ics.window_sequence == EIGHT_SHORT else 5
    ics.sfb_cb = []
    ics.sections = []
    for g in range(ics.num_groups):
        cbs = []
        secs = []
        k = 0
        while k < ics.max_sfb:
            cb = br.bits(4)
            if cb == 12:
                raise AacError('reserved codebook 12')
            ln = 0
            while True:
                incr = br.bits(nb)
                ln += incr
                if incr != esc:
                    break
            if ln == 0 or k + ln > ics.max_sfb:
                raise AacError('section runs past max_sfb')
            secs.append((cb, k, k + ln))
            cbs += [cb] * ln
            k += ln
        ics.sfb_cb.append(cbs)
        ics.sections.append(secs)


def wrap12(v):
    """Intensity positions and noise energies are held in twelve bits by the
    RTL and wrap there; no legal stream comes near it."""
    return ((v + 2048) & 4095) - 2048


def parse_scale_factor_data(br, ics):
    """Returns per (g, sfb) the decoded value: a scalefactor, an intensity
    position or a noise energy, by the band's codebook (4.6.2.3.2,
    4.6.8.2.3, 4.6.13.3)."""
    sf = ics.global_gain
    is_pos = 0
    nrg = ics.global_gain - 90 - 256
    noise_pcm = True
    ics.sf = []
    for g in range(ics.num_groups):
        row = []
        for sfb in range(ics.max_sfb):
            cb = ics.sfb_cb[g][sfb]
            if cb == ZERO_HCB:
                row.append(0)
            elif cb in (INTENSITY_HCB, INTENSITY_HCB2):
                is_pos = wrap12(is_pos + br.huff(0) - 60)
                row.append(is_pos)
            elif cb == NOISE_HCB:
                if noise_pcm:
                    noise_pcm = False
                    nrg = wrap12(nrg + br.bits(9))
                else:
                    nrg = wrap12(nrg + br.huff(0) - 60)
                row.append(nrg)
            else:
                sf += br.huff(0) - 60
                if sf < 0 or sf > 255:
                    raise AacError('scalefactor %d out of range' % sf)
                row.append(sf)
        ics.sf.append(row)


def parse_pulse_data(br, ics):
    ics.pulse_num = br.bits(2) + 1
    ics.pulse_start = br.bits(6)
    ics.pulse = [(br.bits(5), br.bits(4)) for _ in range(ics.pulse_num)]


def parse_tns_data(br, ics):
    short = ics.window_sequence == EIGHT_SHORT
    ics.tns = []
    for w in range(ics.num_windows):
        n_filt = br.bits(1 if short else 2)
        coef_res = br.bits(1) if n_filt else 0
        filters = []
        for f in range(n_filt):
            length = br.bits(4 if short else 6)
            order = br.bits(3 if short else 5)
            direction = compress = 0
            coefs = []
            if order:
                direction = br.bits(1)
                compress = br.bits(1)
                nbits = coef_res + 3 - compress
                coefs = [br.bits(nbits) for _ in range(order)]
            filters.append((length, order, direction, compress, coefs))
        ics.tns.append((coef_res, filters))


def parse_spectral_data(br, ics, cov):
    """Quantised coefficients, de-interleaved: q[w*128 + k] for short
    windows, q[k] for long (4.6.3.3, quant_to_spec)."""
    q = [0] * 1024
    w0 = 0
    for g in range(ics.num_groups):
        glen = ics.group_len[g]
        # The interleaved order of this group, as de-interleaved addresses.
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
            if cb in (ZERO_HCB, NOISE_HCB, INTENSITY_HCB, INTENSITY_HCB2):
                continue
            cov.add('cb%d' % cb)
            uns, dim, lav = CB_PARAMS[cb]
            k = sfb_start[start]
            while k < sfb_start[end]:
                idx = br.huff(cb)
                mod = lav + 1 if uns else 2 * lav + 1
                off = 0 if uns else lav
                vals = []
                for d in range(dim):
                    p = mod ** (dim - 1 - d)
                    vals.append(idx // p - off)
                    idx %= p
                if uns:
                    for d in range(dim):
                        if vals[d] and br.bits(1):
                            vals[d] = -vals[d]
                if cb == ESC_HCB:
                    for d in range(dim):
                        if abs(vals[d]) == 16:
                            n = 0
                            while br.bits(1):
                                n += 1
                                if n > 8:
                                    raise AacError('escape prefix too long')
                            v = (1 << (n + 4)) + br.bits(n + 4)
                            cov.add('esc_values')
                            if v > 1023:
                                cov.add('esc_over_1023')
                            vals[d] = -v if vals[d] < 0 else v
                for d in range(dim):
                    q[order[k + d]] = vals[d]
                k += dim
        w0 += glen
    return q


def apply_pulses(ics, q):
    k = ics.swb[ics.pulse_start]
    for off, amp in ics.pulse:
        k += off
        if k >= 1024:
            raise AacError('pulse past the spectrum')
        if q[k] > 0:
            q[k] += amp
        else:
            q[k] -= amp


def parse_ics(br, ics, sfi, common_window, cov):
    ics.global_gain = br.bits(8)
    if not common_window:
        parse_ics_info(br, ics, sfi)
    parse_section_data(br, ics)
    parse_scale_factor_data(br, ics)
    ics.pulse_present = br.bits(1)
    if ics.pulse_present:
        if ics.window_sequence == EIGHT_SHORT:
            raise AacError('pulse data in a short window')
        parse_pulse_data(br, ics)
        if ics.pulse_start > ics.num_swb:
            raise AacError('pulse_start_sfb past num_swb')
        cov.add('pulse')
    ics.tns_present = br.bits(1)
    if ics.tns_present:
        parse_tns_data(br, ics)
        cov.add('tns')
        if ics.window_sequence == EIGHT_SHORT:
            cov.add('tns_short')
        for coef_res, filters in ics.tns:
            if not coef_res and filters:
                cov.add('tns_res3')
            for length, order, direction, compress, coefs in filters:
                if order and direction:
                    cov.add('tns_downward')
                if order and compress:
                    cov.add('tns_compress')
    if br.bits(1):
        raise AacError('gain_control_data_present in AAC LC')
    ics.q = parse_spectral_data(br, ics, cov)
    ics.qc = list(ics.q)   # as coded, before the pulses
    if ics.pulse_present:
        apply_pulses(ics, ics.q)
    cov.add(['only_long', 'long_start', 'eight_short',
             'long_stop'][ics.window_sequence])
    cov.add('shape_kbd' if ics.window_shape else 'shape_sine')
    if ics.window_sequence == EIGHT_SHORT and ics.num_groups < 8:
        cov.add('grouped_short')
    for g in range(ics.num_groups):
        for sfb in range(ics.max_sfb):
            cb = ics.sfb_cb[g][sfb]
            if cb == NOISE_HCB:
                cov.add('noise_bands')
            if cb in (INTENSITY_HCB, INTENSITY_HCB2):
                cov.add('intensity_bands')
            if cb == INTENSITY_HCB2:
                cov.add('intensity_hcb2')


class Element:
    pass


def parse_raw_data_block(br, sfi, channels, cov):
    """One raw_data_block: a list of audio elements (SCE or CPE) in order.
    FIL, DSE and PCE are skipped; CCE and LFE are refused."""
    elements = []
    while True:
        eid = br.bits(3)
        if eid == ID_END:
            break
        if eid == ID_SCE:
            el = Element()
            el.kind = ID_SCE
            el.tag = br.bits(4)
            ics = Ics()
            parse_ics(br, ics, sfi, False, cov)
            el.ics = [ics]
            elements.append(el)
            cov.add('sce')
        elif eid == ID_CPE:
            el = Element()
            el.kind = ID_CPE
            el.tag = br.bits(4)
            el.common_window = br.bits(1)
            a, b = Ics(), Ics()
            el.ms_mask = 0
            el.ms_used = None
            if el.common_window:
                cov.add('common_window')
                parse_ics_info(br, a, sfi)
                for k in ('window_sequence', 'window_shape', 'max_sfb',
                          'grouping'):
                    setattr(b, k, getattr(a, k))
                window_info(b, sfi)
                el.ms_mask = br.bits(2)
                if el.ms_mask == 3:
                    raise AacError('reserved ms_mask_present 3')
                if el.ms_mask == 1:
                    el.ms_used = [[br.bits(1) for _ in range(a.max_sfb)]
                                  for _ in range(a.num_groups)]
                    cov.add('ms_mask_1')
                    cov.add('ms_bands', sum(map(sum, el.ms_used)))
                elif el.ms_mask == 2:
                    cov.add('ms_mask_2')
            parse_ics(br, a, sfi, el.common_window, cov)
            parse_ics(br, b, sfi, el.common_window, cov)
            el.ics = [a, b]
            elements.append(el)
            cov.add('cpe')
            if a.window_sequence != b.window_sequence:
                cov.add('independent_windows')
            for g in range(b.num_groups if el.common_window else 0):
                for sfb in range(b.max_sfb):
                    if (a.sfb_cb[g][sfb] == NOISE_HCB and
                            b.sfb_cb[g][sfb] == NOISE_HCB and
                            (el.ms_mask == 2 or
                             (el.ms_mask == 1 and el.ms_used[g][sfb]))):
                        cov.add('noise_correlated')
        elif eid == ID_DSE:
            br.bits(4)
            align = br.bits(1)
            cnt = br.bits(8)
            if cnt == 255:
                cnt += br.bits(8)
            if align:
                br.align()
            br.pos += 8 * cnt
            cov.add('dse')
        elif eid == ID_FIL:
            cnt = br.bits(4)
            if cnt == 15:
                cnt += br.bits(8) - 1
            br.pos += 8 * cnt
            cov.add('fil')
        elif eid == ID_PCE:
            skip_pce(br)
            cov.add('pce')
        else:
            raise AacError('unsupported element %d' % eid)
    br.align()
    nch = sum(1 if e.kind == ID_SCE else 2 for e in elements)
    if nch != channels or len(elements) != 1:
        raise AacError('%d channels in %d elements, %d expected' %
                       (nch, len(elements), channels))
    return elements


def skip_pce(br):
    """program_config_element (Table 4.2), parsed only to be skipped. The
    byte_alignment is relative to the raw_data_block, which starts on a
    byte, so it is an ordinary alignment here."""
    br.bits(4 + 2 + 4)
    nf, ns, nb = br.bits(4), br.bits(4), br.bits(4)
    nl, na, nc = br.bits(2), br.bits(3), br.bits(4)
    if br.bits(1):
        br.bits(4)
    if br.bits(1):
        br.bits(4)
    if br.bits(1):
        br.bits(3)
    br.bits(5 * (nf + ns + nb) + 4 * nl + 4 * na + 5 * nc)
    br.align()
    br.bits(8 * br.bits(8))


def parse_adts(data, cov=None):
    """Yields (sfi, channels, elements) for each raw_data_block of an ADTS
    stream (1.A.3)."""
    cov = cov or Coverage()
    pos = 0
    while pos + 7 <= len(data):
        br = BitReader(data, pos * 8)
        if br.bits(12) != 0xFFF:
            raise AacError('lost ADTS sync at byte %d' % pos)
        br.bits(1 + 2)  # ID, layer
        protection_absent = br.bits(1)
        profile = br.bits(2)
        sfi = br.bits(4)
        br.bits(1)
        ch = br.bits(3)
        br.bits(1 + 1 + 1 + 1)
        flen = br.bits(13)
        br.bits(11)
        nrdb = br.bits(2)
        if profile != 1:
            raise AacError('profile %d is not AAC LC' % profile)
        if not protection_absent:
            cov.add('adts_crc')
            if nrdb:
                br.bits(16 * nrdb)
            br.bits(16)
        if nrdb:
            cov.add('multi_rdb')
        for i in range(nrdb + 1):
            els = parse_raw_data_block(br, sfi, ch, cov)
            cov.add('frames')
            yield sfi, ch, els
            if nrdb and not protection_absent:
                br.bits(16)
        pos += flen


def parse_raw(units, sfi, ch, cov=None):
    cov = cov or Coverage()
    for au in units:
        br = BitReader(au)
        els = parse_raw_data_block(br, sfi, ch, cov)
        cov.add('frames')
        yield sfi, ch, els

# ---------------------------------------------------------------------------
# Windows (4.6.11.3.2), from their formulas
# ---------------------------------------------------------------------------


def bessel_i0(x):
    s, term, k = 1.0, 1.0, 1
    while term > 1e-30 * s:
        term *= (x / 2.0 / k) ** 2
        s += term
        k += 1
    return s


def kbd_left(n_win, alpha):
    """W_KBD_LEFT,N(n) for 0 <= n < N/2."""
    half = n_win // 2
    kern = [bessel_i0(math.pi * alpha *
                      math.sqrt(1.0 - ((p - n_win / 4.0) / (n_win / 4.0)) ** 2))
            / bessel_i0(math.pi * alpha) for p in range(half + 1)]
    total = sum(kern)
    out, acc = [], 0.0
    for n in range(half):
        acc += kern[n]
        out.append(math.sqrt(acc / total))
    return out


def sine_left(n_win):
    return [math.sin(math.pi / n_win * (n + 0.5)) for n in range(n_win // 2)]


WIN_LEFT = {
    (2048, 0): sine_left(2048), (2048, 1): kbd_left(2048, 4.0),
    (256, 0): sine_left(256), (256, 1): kbd_left(256, 6.0),
}


def window_long(seq, shape, prev_shape):
    """The 2048-point window of a long window_sequence, a)/b)/d)."""
    left_l = WIN_LEFT[(2048, prev_shape)]
    right_l = WIN_LEFT[(2048, shape)][::-1]
    left_s = WIN_LEFT[(256, prev_shape)]
    right_s = WIN_LEFT[(256, shape)][::-1]
    w = [0.0] * 2048
    if seq == LONG_STOP:
        for n in range(448, 576):
            w[n] = left_s[n - 448]
        for n in range(576, 1024):
            w[n] = 1.0
    else:
        w[:1024] = left_l
    if seq == LONG_START:
        for n in range(1024, 1472):
            w[n] = 1.0
        for n in range(1472, 1600):
            w[n] = right_s[n - 1472]
    else:
        w[1024:] = right_l
    return w

# ---------------------------------------------------------------------------
# The reference decoder, in double precision
# ---------------------------------------------------------------------------


def band_iter(ics):
    """Yields (g, win, sfb, first coefficient address, width) in the order
    the standard's loops visit them, de-interleaved."""
    w0 = 0
    for g in range(ics.num_groups):
        for win in range(ics.group_len[g]):
            for sfb in range(ics.max_sfb):
                width = ics.swb[sfb + 1] - ics.swb[sfb]
                yield g, win, sfb, (w0 + win) * 128 + ics.swb[sfb], width
        w0 += ics.group_len[g]


def tns_lpc(order, coef_res, compress, coefs):
    """tns_decode_coef: the reflection coefficients and the LPC from them."""
    bits = coef_res + 3
    res2 = bits - compress
    iqfac = ((1 << (bits - 1)) - 0.5) / (math.pi / 2.0)
    iqfac_m = ((1 << (bits - 1)) + 0.5) / (math.pi / 2.0)
    refl = []
    for c in coefs[:order]:
        t = c - (1 << res2) if c & (1 << (res2 - 1)) else c
        refl.append(math.sin(t / (iqfac if t >= 0 else iqfac_m)))
    a = [1.0] + [0.0] * order
    for m in range(1, order + 1):
        b = a[:]
        for i in range(1, m):
            b[i] = a[i] + refl[m - 1] * a[m - i]
        a = b
        a[m] = refl[m - 1]
    return refl, a


def tns_ranges(ics):
    """(window, start, size, inc, reflection coefficients, order) for each
    filter tns_decode_frame applies, in its order."""
    out = []
    max_order = TNS_MAX_ORDER_SHORT if ics.num_windows == 8 else \
        TNS_MAX_ORDER_LONG
    for w in range(ics.num_windows):
        coef_res, filters = ics.tns[w]
        bottom = ics.num_swb
        for length, order, direction, compress, coefs in filters:
            top = bottom
            bottom = max(top - length, 0)
            tns_order = min(order, max_order)
            if not tns_order:
                continue
            start = ics.swb[min(bottom, ics.tns_max_bands, ics.max_sfb)]
            end = ics.swb[min(top, ics.tns_max_bands, ics.max_sfb)]
            size = end - start
            if size <= 0:
                continue
            if direction:
                inc, first = -1, end - 1
            else:
                inc, first = 1, start
            out.append((w, first, size, inc, (tns_order, coef_res,
                                                compress, coefs)))
    return out


class FloatDecoder:
    def __init__(self, seed=1):
        self.prev_shape = [0, 0]
        self.overlap = [[0.0] * 1024, [0.0] * 1024]
        self.first = True
        self.rnd = seed

    def rand(self):
        self.rnd = (self.rnd * 1664525 + 1013904223) & 0xFFFFFFFF
        return (self.rnd / 2.0 ** 31) - 1.0

    def dequant(self, ics):
        x = [0.0] * 1024
        for g, win, sfb, base, width in band_iter(ics):
            cb = ics.sfb_cb[g][sfb]
            if cb in (ZERO_HCB, NOISE_HCB, INTENSITY_HCB, INTENSITY_HCB2):
                continue
            gain = 2.0 ** (0.25 * (ics.sf[g][sfb] - 100))
            for k in range(base, base + width):
                v = ics.q[k]
                x[k] = math.copysign(abs(v) ** (4.0 / 3.0), v) * gain
        return x

    def noise(self, ics, x, g, sfb, base, width, vec=None):
        if vec is None:
            vec = [self.rand() for _ in range(width)]
        nrg = sum(v * v for v in vec)
        scale = 2.0 ** (0.25 * ics.sf[g][sfb]) / math.sqrt(nrg)
        for i in range(width):
            x[base + i] = vec[i] * scale
        return vec

    def stereo(self, el, xs):
        a = el.ics[0]
        if el.kind == ID_SCE:
            for g, win, sfb, base, width in band_iter(a):
                if a.sfb_cb[g][sfb] == NOISE_HCB:
                    self.noise(a, xs[0], g, sfb, base, width)
            return
        b = el.ics[1]
        l, r = xs
        for ch, ics in ((0, a), (1, b)):
            for g, win, sfb, base, width in band_iter(ics):
                if ics.sfb_cb[g][sfb] != NOISE_HCB:
                    continue
                if ch == 1 and el.common_window and \
                        a.sfb_cb[g][sfb] == NOISE_HCB and self.ms(el, g, sfb):
                    vec = [l[base + i] for i in range(width)]
                    self.noise(ics, r, g, sfb, base, width, vec)
                else:
                    self.noise(ics, xs[ch], g, sfb, base, width)
        if not el.common_window:
            return
        for g, win, sfb, base, width in band_iter(a):
            cb_r = b.sfb_cb[g][sfb]
            is_i = cb_r in (INTENSITY_HCB, INTENSITY_HCB2)
            is_n = a.sfb_cb[g][sfb] == NOISE_HCB or cb_r == NOISE_HCB
            if self.ms(el, g, sfb) and not is_i and not is_n:
                for k in range(base, base + width):
                    l[k], r[k] = l[k] + r[k], l[k] - r[k]
        for g, win, sfb, base, width in band_iter(b):
            cb_r = b.sfb_cb[g][sfb]
            if cb_r not in (INTENSITY_HCB, INTENSITY_HCB2):
                continue
            sign = 1 if cb_r == INTENSITY_HCB else -1
            if el.ms_mask == 1 and el.ms_used[g][sfb]:
                sign = -sign
            scale = sign * 0.5 ** (0.25 * b.sf[g][sfb])
            for k in range(base, base + width):
                r[k] = scale * l[k]

    @staticmethod
    def ms(el, g, sfb):
        return el.ms_mask == 2 or (el.ms_mask == 1 and el.ms_used[g][sfb])

    def tns(self, ics, x):
        if not ics.tns_present:
            return
        for w, first, size, inc, params in tns_ranges(ics):
            refl, a = tns_lpc(*params)
            order = len(a) - 1
            hist = [0.0] * order
            k = w * 128 + first
            for _ in range(size):
                y = x[k]
                for i in range(order):
                    y -= a[i + 1] * hist[i]
                hist = [y] + hist[:-1]
                x[k] = y
                k += inc

    @staticmethod
    def imdct(spec, n_win):
        """x[n] = 2/N sum spec[k] cos(2pi/N (n + n0)(k + 1/2)), direct."""
        import numpy as np
        m = n_win // 2
        n0 = (m + 1) / 2.0
        n = np.arange(n_win)[:, None]
        k = np.arange(m)[None, :]
        c = np.cos(2 * np.pi / n_win * (n + n0) * (k + 0.5))
        return list((2.0 / n_win) * c.dot(np.array(spec, dtype=float)))

    def synth(self, ch, ics, x):
        prev = self.prev_shape[ch] if not self.first else ics.window_shape
        if ics.window_sequence == EIGHT_SHORT:
            z = [0.0] * 2048
            for j in range(8):
                xs = self.imdct(x[j * 128:(j + 1) * 128], 256)
                lshape = prev if j == 0 else ics.window_shape
                wl = WIN_LEFT[(256, lshape)]
                wr = WIN_LEFT[(256, ics.window_shape)][::-1]
                win = wl + wr
                for n in range(256):
                    z[448 + 128 * j + n] += win[n] * xs[n]
        else:
            xs = self.imdct(x, 2048)
            w = window_long(ics.window_sequence, ics.window_shape, prev)
            z = [w[n] * xs[n] for n in range(2048)]
        out = [z[n] + self.overlap[ch][n] for n in range(1024)]
        self.overlap[ch] = z[1024:]
        self.prev_shape[ch] = ics.window_shape
        return out

    def decode(self, sfi, ch, elements):
        el = elements[0]
        xs = [self.dequant(ics) for ics in el.ics]
        self.stereo(el, xs)
        outs = []
        for c, ics in enumerate(el.ics):
            self.tns(ics, xs[c])
            outs.append(self.synth(c, ics, xs[c]))
        self.first = False
        return outs


def to_s16(v):
    r = int(math.floor(v + 0.5))
    return max(-32768, min(32767, r))


# ---------------------------------------------------------------------------
# The bit-accurate model of the RTL
# ---------------------------------------------------------------------------
#
# Number formats (the RTL's, exactly):
#
#   spectral value  35-bit signed, FX = 7 fractional bits, in the units of the
#                   standard's x_rescal (so a value of 1.0 is 128)
#   coefficient     25-bit signed, CF = 23 fractional bits (Q1.23): windows,
#                   twiddles, gain mantissas, TNS reflection coefficients
#   |q|^(4/3)       34-bit unsigned, 20 fractional bits, for |q| <= 1024
#
# Every product is a 35-bit by 25-bit signed multiply, which is what two
# DSP48E1s make. Every rounding is "add half, shift right arithmetically"
# (round half up), and every narrowing saturates.

FX = 7
XW = 35
CF = 23
POW_FRAC = 20


def rshift_round(v, r):
    if r <= 0:
        return v << -r
    return (v + (1 << (r - 1))) >> r


def sat(v, bits):
    hi = (1 << (bits - 1)) - 1
    return hi if v > hi else (-hi - 1 if v < -hi - 1 else v)


def qcoef(v):
    """A real number as a Q1.23 coefficient."""
    return int(math.floor(v * (1 << CF) + 0.5))


# 2^(f/4), f = 0..3: the gain mantissas of 4.6.2.3.3, 4.6.8.2.3, 4.6.13.3.
GAIN_MANT = [qcoef(2.0 ** (f / 4.0)) for f in range(4)]
# |q|^(4/3) for 0..1024 (4.6.1.3), 20 fractional bits.
POW43 = [int(math.floor(i ** (4.0 / 3.0) * (1 << POW_FRAC) + 0.5))
         for i in range(1025)]
# 1/sqrt(width) for the band widths 4, 8, .. 96 (index width/4 - 1).
INV_SQRT = [qcoef(1.0 / math.sqrt(4 * (i + 1))) for i in range(24)]


def tns_refl_table():
    """sin(t / iqfac) for every transmitted index t, 4.6.9.3, as Q1.23:
    entry [coef_res][t & 15]."""
    tab = [[0] * 16, [0] * 16]
    for res in (0, 1):
        bits = res + 3
        iqfac = ((1 << (bits - 1)) - 0.5) / (math.pi / 2.0)
        iqfac_m = ((1 << (bits - 1)) + 0.5) / (math.pi / 2.0)
        for t in range(-(1 << (bits - 1)), 1 << (bits - 1)):
            tab[res][t & 15] = qcoef(math.sin(t / (iqfac if t >= 0
                                                    else iqfac_m)))
    return tab


TNS_REFL = tns_refl_table()


def twiddles(m):
    """exp(-i pi (n + 1/8) / M) for n < M/2: the pre- and post-twiddle."""
    return [(qcoef(math.cos(math.pi * (n + 0.125) / m)),
             qcoef(-math.sin(math.pi * (n + 0.125) / m)))
            for n in range(m // 2)]


TW_PRE = {1024: twiddles(1024), 128: twiddles(128)}
# exp(-2 pi i k / 512) for k < 256; the 64-point FFT takes every eighth.
TW_FFT = [(qcoef(math.cos(2 * math.pi * k / 512)),
           qcoef(-math.sin(2 * math.pi * k / 512))) for k in range(256)]
WIN_Q = {key: [qcoef(v) for v in vals] for key, vals in WIN_LEFT.items()}
ONE = 1 << CF


def bitrev(v, bits):
    r = 0
    for _ in range(bits):
        r = (r << 1) | (v & 1)
        v >>= 1
    return r


def pow43_fixed(a):
    """(mantissa, extra left shift in quarter-gain steps / 4) of
    |q|^(4/3) * 2^20 for |q| <= 8191. Above 1024 it is interpolated:
    q = 8j + r, (8j + r)^(4/3) = 16 (j + r/8)^(4/3)."""
    if a <= 1024:
        return POW43[a], 0
    j, r = a >> 3, a & 7
    d = POW43[j + 1] - POW43[j]
    return POW43[j] + ((r * d + 4) >> 3), 4


def scale_gain(mant, g, frac_bits):
    """mant * 2^(g/4), mant having frac_bits fractional bits, as a spectral
    value: the 35x25 product with GAIN_MANT, shifted and saturated."""
    e, f = g >> 2, g & 3
    sh = frac_bits + CF - FX - e
    if sh >= 64:
        return 0
    return sat(rshift_round(mant * GAIN_MANT[f], sh), XW)


def mul_round(a, c, r=CF):
    """A 35-bit value times a Q1.23 coefficient, rounded to 35 bits."""
    return sat(rshift_round(a * c, r), XW)


class Prbs31:
    """x^31 + x^28 + 1, one bit a step: the PNS sign generator."""

    def __init__(self):
        self.s = 1

    def step(self):
        b = ((self.s >> 30) ^ (self.s >> 27)) & 1
        self.s = ((self.s << 1) | b) & 0x7FFFFFFF
        return b


class FixedDecoder:
    def __init__(self):
        self.prev_shape = [0, 0]
        self.tail = [[0] * 1024, [0] * 1024]
        self.first = True
        self.prbs = Prbs31()

    # -- dequantisation, 4.6.1 and 4.6.2 ----------------------------------
    def dequant(self, ics):
        x = [0] * 1024
        for g, win, sfb, base, width in band_iter(ics):
            cb = ics.sfb_cb[g][sfb]
            if cb in (ZERO_HCB, NOISE_HCB, INTENSITY_HCB, INTENSITY_HCB2):
                continue
            gexp = ics.sf[g][sfb] - 100
            for k in range(base, base + width):
                q = ics.q[k]
                a = min(abs(q), 8191)
                mant, shift = pow43_fixed(a)
                v = scale_gain(mant, gexp + 4 * shift, POW_FRAC)
                x[k] = -v if q < 0 else v
        return x

    # -- PNS, M/S, intensity: one pass, band by band ------------------------
    def noise_band(self, ics, x, g, sfb, base, width, state=None):
        """Writes the band with +-V, V = 2^(nrg/4) / sqrt(width). Returns the
        generator state the band started from."""
        start = self.prbs.s
        if state is not None:
            self.prbs.s = state
        nrg = ics.sf[g][sfb]
        e = nrg >> 2
        sh = 2 * CF - FX - e
        prod = GAIN_MANT[nrg & 3] * INV_SQRT[width // 4 - 1]
        v = sat(rshift_round(prod, sh), XW) if sh < 64 else 0
        for i in range(width):
            x[base + i] = -v if self.prbs.step() else v
        return start

    def stereo(self, el, xs):
        a = el.ics[0]
        if el.kind == ID_SCE or not el.common_window:
            for c, ics in enumerate(el.ics):
                for g, win, sfb, base, width in band_iter(ics):
                    if ics.sfb_cb[g][sfb] == NOISE_HCB:
                        self.noise_band(ics, xs[c], g, sfb, base, width)
            return
        b = el.ics[1]
        l, r = xs
        for g, win, sfb, base, width in band_iter(a):
            cl, cr = a.sfb_cb[g][sfb], b.sfb_cb[g][sfb]
            ms = el.ms_mask == 2 or (el.ms_mask == 1 and el.ms_used[g][sfb])
            st = None
            if cl == NOISE_HCB:
                st = self.noise_band(a, l, g, sfb, base, width)
            if cr == NOISE_HCB:
                corr = st if (cl == NOISE_HCB and ms) else None
                self.noise_band(b, r, g, sfb, base, width, corr)
            if cr in (INTENSITY_HCB, INTENSITY_HCB2):
                neg = (cr == INTENSITY_HCB2) != (el.ms_mask == 1 and ms)
                gexp = -b.sf[g][sfb]
                mant = GAIN_MANT[gexp & 3]
                sh = CF - (gexp >> 2)
                for k in range(base, base + width):
                    v = sat(rshift_round(l[k] * mant, sh), XW) \
                        if sh < 64 else 0
                    r[k] = sat(-v, XW) if neg else v
            elif ms and cl != NOISE_HCB and cr != NOISE_HCB:
                for k in range(base, base + width):
                    l[k], r[k] = sat(l[k] + r[k], XW), sat(l[k] - r[k], XW)

    # -- TNS, 4.6.9, as a lattice ------------------------------------------
    def tns(self, ics, x):
        if not ics.tns_present:
            return
        for w, first, size, inc, (order, res, compress, coefs) in \
                tns_ranges(ics):
            res2 = res + 3 - compress
            refl = []
            for c in coefs[:order]:
                t = c - (1 << res2) if c & (1 << (res2 - 1)) else c
                refl.append(TNS_REFL[res][t & 15])
            g = [0] * order
            k = w * 128 + first
            for _ in range(size):
                f = x[k]
                for m in range(order, 0, -1):
                    f = sat(f - rshift_round(refl[m - 1] * g[m - 1], CF), XW)
                    if m < order:
                        g[m] = sat(rshift_round(refl[m - 1] * f, CF) +
                                   g[m - 1], XW)
                g[0] = f
                x[k] = f
                k += inc

    # -- IMDCT, 4.6.11.3.1, by an M/2-point FFT ------------------------------
    @staticmethod
    def imdct(x, base, m):
        """D[0..M-1], the DCT-IV of x[base..base+M) divided by M, from which
        imdct_sample reads the 2M IMDCT outputs. The pre-twiddle halves, and
        each FFT stage halves, which together are the 1/M."""
        q = m // 2
        bits = q.bit_length() - 1
        tw = TW_PRE[m]
        re = [0] * q
        im = [0] * q
        for n in range(q):
            a, b = x[base + 2 * n], x[base + m - 1 - 2 * n]
            wr, wi = tw[n]
            p = bitrev(n, bits)
            re[p] = sat(rshift_round(a * wr - b * wi, CF + 1), XW)
            im[p] = sat(rshift_round(a * wi + b * wr, CF + 1), XW)
        half = 1
        while half < q:
            step = 256 // half
            for start in range(0, q, 2 * half):
                for j in range(half):
                    wr, wi = TW_FFT[j * step]
                    t0, t1 = start + j, start + j + half
                    br, bi = re[t1], im[t1]
                    tr = sat(rshift_round(br * wr - bi * wi, CF), XW)
                    ti = sat(rshift_round(br * wi + bi * wr, CF), XW)
                    ar, ai = re[t0], im[t0]
                    re[t0] = sat((ar + tr + 1) >> 1, XW)
                    im[t0] = sat((ai + ti + 1) >> 1, XW)
                    re[t1] = sat((ar - tr + 1) >> 1, XW)
                    im[t1] = sat((ai - ti + 1) >> 1, XW)
            half *= 2
        d = [0] * m
        for k in range(q):
            wr, wi = tw[k]
            yr = sat(rshift_round(re[k] * wr - im[k] * wi, CF), XW)
            yi = sat(rshift_round(re[k] * wi + im[k] * wr, CF), XW)
            d[2 * k] = yr
            d[m - 1 - 2 * k] = sat(-yi, XW)
        return d

    @staticmethod
    def imdct_sample(d, m, n):
        """The IMDCT output x[n], 0 <= n < 2M, from D."""
        p = n + m // 2
        if p < m:
            return d[p]
        if p < 2 * m:
            return sat(-d[2 * m - 1 - p], XW)
        return sat(-d[p - 2 * m], XW)

    # -- windowing and overlap-add, 4.6.11.3.2 and .3 -------------------------
    @staticmethod
    def long_window(seq, shape, prev, n):
        if n < 1024:
            if seq == LONG_STOP:
                if n < 448:
                    return 0
                if n < 576:
                    return WIN_Q[(256, prev)][n - 448]
                return ONE
            return WIN_Q[(2048, prev)][n]
        n2 = 2047 - n
        if seq == LONG_START:
            if n2 < 448:
                return 0
            if n2 < 576:
                return WIN_Q[(256, shape)][n2 - 448]
            return ONE
        return WIN_Q[(2048, shape)][n2]

    def synth(self, ch, ics, x):
        prev = self.prev_shape[ch] if not self.first else ics.window_shape
        z = [0] * 2048
        if ics.window_sequence == EIGHT_SHORT:
            for j in range(8):
                d = self.imdct(x, j * 128, 128)
                lshape = prev if j == 0 else ics.window_shape
                for n in range(256):
                    w = WIN_Q[(256, lshape)][n] if n < 128 else \
                        WIN_Q[(256, ics.window_shape)][255 - n]
                    v = self.imdct_sample(d, 128, n)
                    p = 448 + 128 * j + n
                    z[p] = sat(z[p] + mul_round(v, w), XW)
        else:
            d = self.imdct(x, 0, 1024)
            for n in range(2048):
                w = self.long_window(ics.window_sequence, ics.window_shape,
                                     prev, n)
                z[n] = mul_round(self.imdct_sample(d, 1024, n), w)
        out = []
        for n in range(1024):
            o = sat(z[n] + self.tail[ch][n], XW)
            out.append(sat(rshift_round(o, FX), 16))
        self.tail[ch] = z[1024:]
        self.prev_shape[ch] = ics.window_shape
        return out

    def decode(self, sfi, ch, elements):
        el = elements[0]
        xs = [self.dequant(ics) for ics in el.ics]
        self.stereo(el, xs)
        outs = []
        for c, ics in enumerate(el.ics):
            self.tns(ics, xs[c])
            outs.append(self.synth(c, ics, xs[c]))
        self.first = False
        return outs


# ---------------------------------------------------------------------------
# The generated ROMs of the RTL
def verilog_case_table(name, comment, in_bits, out_bits, values, signed=True):
    lines = ['// Generated by tools/aac_dec_model.py tables: do not edit.',
             '//']
    lines += ['// ' + c for c in comment]
    lines += ['module %s (' % name,
              '    input  wire [%d:0] idx,' % (in_bits - 1),
              '    output reg  [%d:0] q' % (out_bits - 1),
              ');',
              '    always @(*) begin',
              '        case (idx)']
    mask = (1 << out_bits) - 1
    for i, v in enumerate(values):
        lines.append("            %d'd%d: q = %d'h%x;" % (in_bits, i, out_bits,
                                                         v & mask))
    lines += ["            default: q = %d'h0;" % out_bits, '        endcase',
              '    end', 'endmodule']
    return '\n'.join(lines) + '\n'


def write_small_tables(rtl):
    with open(os.path.join(rtl, 'aac_gain_mant.v'), 'w') as f:
        f.write(verilog_case_table(
            'aac_gain_mant', ['2^(f/4) for f = 0..3 as Q1.23: the mantissa of every',
                              'quarter-power-of-two gain (4.6.2.3.3, 4.6.8.2.3, 4.6.13.3).'],
            2, 25, GAIN_MANT))
    with open(os.path.join(rtl, 'aac_inv_sqrt.v'), 'w') as f:
        f.write(verilog_case_table(
            'aac_inv_sqrt', ['1/sqrt(4 (idx + 1)) as Q1.23: the PNS normalisation of a',
                             'band of that width whose values are all +-1.'],
            5, 25, INV_SQRT))
    refl = TNS_REFL[0] + TNS_REFL[1]
    with open(os.path.join(rtl, 'aac_tns_refl.v'), 'w') as f:
        f.write(verilog_case_table(
            'aac_tns_refl', ['The TNS reflection coefficients of 4.6.9.3 as Q1.23,',
                             'addressed {coef_res, t[3:0]}: sin(t / iqfac) for t >= 0,',
                             'sin(t / iqfac_m) for t < 0.'],
            5, 25, refl))


# ---------------------------------------------------------------------------

HUFF_LEAF = 1 << 11


def huff_payload(book, idx):
    """What a leaf of `book` holds for codeword index `idx`: the scalefactor
    index itself for book 0, else the n-tuple already split into fields, so
    the hardware never divides (4.6.3.3's INT(idx/mod) done here)."""
    if book == 0:
        return idx
    uns, dim, lav = CB_PARAMS[book]
    mod = lav + 1 if uns else 2 * lav + 1
    off = 0 if uns else lav
    vals = []
    for d in range(dim):
        p = mod ** (dim - 1 - d)
        vals.append(idx // p - off)
        idx %= p
    if dim == 4:
        out = 0
        for v in vals:
            out = (out << 2) | (v & 3)
        return out
    return ((vals[0] & 31) << 5) | (vals[1] & 31)


def huff_tree():
    """(entries, roots): entry[2 * node + bit] is HUFF_LEAF | payload, or the
    child node's number. Book b's root is node roots[b]."""
    entries = {}
    roots = []
    next_node = [0]

    def new_node():
        n = next_node[0]
        next_node[0] += 1
        return n

    for book in range(12):
        root = new_node()
        roots.append(root)
        children = {}
        for idx, (ln, cw) in sorted(CODEBOOKS[book].items()):
            node = root
            for i in range(ln - 1, -1, -1):
                bit = (cw >> i) & 1
                key = 2 * node + bit
                if i == 0:
                    assert key not in entries and key not in children
                    entries[key] = HUFF_LEAF | huff_payload(book, idx)
                else:
                    if key not in children:
                        assert key not in entries
                        children[key] = new_node()
                        entries[key] = children[key]
                    node = children[key]
    assert next_node[0] * 2 == len(entries), 'every code is complete'
    return [entries[i] for i in range(len(entries))], roots


def win_rom():
    """Left window halves: sine 2048, KBD 2048, sine 256, KBD 256."""
    return (WIN_Q[(2048, 0)] + WIN_Q[(2048, 1)] + WIN_Q[(256, 0)] +
            WIN_Q[(256, 1)])


def tw_rom():
    """pre/post twiddles M=1024 (512), M=128 (64), then the FFT's 256."""
    return TW_PRE[1024] + TW_PRE[128] + TW_FFT


def verilog_rom(name, width, depth, values, comment, ports=1):
    lines = ['// Generated by tools/aac_dec_model.py tables: do not edit.',
             '//']
    lines += ['// ' + c for c in comment]
    abits = (depth - 1).bit_length()
    depth = len(values)
    lines.append('module %s (' % name)
    lines.append('    input  wire             clk,')
    if ports == 1:
        lines.append('    input  wire [%2d:0]      addr,' % (abits - 1))
        lines.append('    output reg  [%2d:0]      q' % (width - 1))
    else:
        lines.append('    input  wire [%2d:0]      addr_a,' % (abits - 1))
        lines.append('    input  wire [%2d:0]      addr_b,' % (abits - 1))
        lines.append('    output reg  [%2d:0]      q_a,' % (width - 1))
        lines.append('    output reg  [%2d:0]      q_b' % (width - 1))
    lines.append(');')
    lines.append('    reg [%d:0] rom [0:%d];' % (width - 1, depth - 1))
    lines.append('    initial begin')
    mask = (1 << width) - 1
    digits = (width + 3) // 4
    for i in range(depth):
        v = values[i] if i < len(values) else 0
        lines.append("        rom[%d] = %d'h%0*x;" % (i, width, digits,
                                                    v & mask))
    lines.append('    end')
    if ports == 1:
        lines.append('    always @(posedge clk) q <= rom[addr];')
    else:
        lines.append('    always @(posedge clk) begin')
        lines.append('        q_a <= rom[addr_a];')
        lines.append('        q_b <= rom[addr_b];')
        lines.append('    end')
    lines.append('endmodule')
    return '\n'.join(lines) + '\n'


def verilog_huff_root(roots):
    lines = ['// Generated by tools/aac_dec_model.py tables: do not edit.',
             '//',
             '// The node aac_huff_rom holds the root of each codebook at:',
             '// book 0 is the scalefactor codebook, 1..11 the spectrum ones.',
             'module aac_huff_root (',
             '    input  wire [3:0]  book,',
             '    output reg  [10:0] root',
             ');',
             '    always @(*) begin',
             '        case (book)']
    for b, r in enumerate(roots):
        lines.append("            4'd%d: root = 11'd%d;" % (b, r))
    lines += ["            default: root = 11'd0;",
              '        endcase', '    end', 'endmodule']
    return '\n'.join(lines) + '\n'


# The five classes of band table, by sampling frequency index.
RATE_CLASS = {3: 0, 4: 0, 5: 1, 6: 2, 7: 2, 8: 3, 9: 3, 10: 3, 11: 4, 12: 4}
CLASS_TABLES = [(SWB_LONG_48, SWB_SHORT_48), (SWB_LONG_32, SWB_SHORT_48),
                (SWB_LONG_24, SWB_SHORT_24), (SWB_LONG_16, SWB_SHORT_16),
                (SWB_LONG_8, SWB_SHORT_8)]


def verilog_swb():
    lines = ['// Generated by tools/aac_dec_model.py tables: do not edit.',
             '//',
             '// swb_offset_long_window and swb_offset_short_window of ISO/IEC',
             '// 14496-3 Tables 4.129 to 4.141 for the rates up to 48 kHz,',
             '// num_swb, and TNS_MAX_BANDS of Table 4.157 (no PQF).',
             '//',
             '// The offsets are a block RAM read on the clock, two bands at',
             '// once (a band and, typically, the next): off_a and off_b are',
             '// those of band_a and band_b as they were at the last edge. A',
             '// band at or past num_swb reads the closing 1024 or 128.',
             '// num_swb and tns_max are combinational.',
             'module aac_swb (',
             '    input  wire        clk,',
             '    input  wire [3:0]  sfi,',
             '    input  wire        short_win,',
             '    input  wire [5:0]  band_a,',
             '    input  wire [5:0]  band_b,',
             '    output reg  [10:0] off_a,',
             '    output reg  [10:0] off_b,',
             '    output reg  [5:0]  num_swb,',
             '    output reg  [5:0]  tns_max',
             ');',
             '    reg [2:0] cls;',
             '    always @(*) begin',
             '        case (sfi)']
    for sfi in sorted(RATE_CLASS):
        lines.append("            4'd%d: cls = 3'd%d;" % (sfi, RATE_CLASS[sfi]))
    lines += ["            default: cls = 3'd0;", '        endcase']
    lines.append('        case (sfi)')
    for sfi in sorted(RATE_TABLES):
        lines.append("            4'd%d: tns_max = short_win ? 6'd%d : 6'd%d;" %
                     (sfi, RATE_TABLES[sfi][3], RATE_TABLES[sfi][2]))
    lines += ["            default: tns_max = 6'd0;", '        endcase']
    lines.append('        case ({cls, short_win})')
    for c, (lt, st) in enumerate(CLASS_TABLES):
        for s, t in ((0, lt), (1, st)):
            lines.append("            {3'd%d, 1'b%d}: num_swb = 6'd%d;" %
                         (c, s, len(t) - 1))
    lines += ["            default: num_swb = 6'd0;", '        endcase',
              '    end',
              '    // {class, short, band}: class 0 is 44.1 and 48 kHz, 1 is 32,',
              '    // 2 is 22.05 and 24, 3 is 11.025 to 16, 4 is 7.35 and 8.',
              '    reg [10:0] rom [0:639];',
              '    initial begin']
    for c, (lt, st) in enumerate(CLASS_TABLES):
        for s, t in ((0, lt), (1, st)):
            for b in range(64):
                lines.append("        rom[%d] = 11'd%d;" %
                             (c * 128 + s * 64 + b, t[min(b, len(t) - 1)]))
    lines += ['    end',
              '    always @(posedge clk) begin',
              '        off_a <= rom[{cls, short_win, band_a}];',
              '        off_b <= rom[{cls, short_win, band_b}];',
              '    end',
              'endmodule']
    return '\n'.join(lines) + '\n'


def write_tables(rtl):
    write_small_tables(rtl)
    entries, roots = huff_tree()
    with open(os.path.join(rtl, 'aac_huff_rom.v'), 'w') as f:
        f.write(verilog_rom(
            'aac_huff_rom', 12, 4096, entries,
            ['The twelve Huffman codebooks of ISO/IEC 14496-3 Tables 4.A.1 to',
             '4.A.12 as one binary tree each: entry 2 * node + bit is either',
             '{1, payload} for a leaf or {0, child node}. Roots, by book: ' +
             ' '.join(str(r) for r in roots) + '.',
             'A leaf of the scalefactor book holds the codeword index; one of',
             'a spectrum book holds the n-tuple already split: four 2-bit',
             'fields for books 1-4, two 5-bit fields for books 5-11.']))
    with open(os.path.join(rtl, 'aac_huff_root.v'), 'w') as f:
        f.write(verilog_huff_root(roots))
    with open(os.path.join(rtl, 'aac_swb.v'), 'w') as f:
        f.write(verilog_swb())
    with open(os.path.join(rtl, 'aac_pow43_rom.v'), 'w') as f:
        f.write(verilog_rom(
            'aac_pow43_rom', 34, 1024, POW43[:1024],
            ['i^(4/3) * 2^20, rounded, for i = 0..1023 (4.6.1.3); 1024^(4/3)',
             'is the constant in aac_dequant. Two read ports, for the two',
             'neighbours that interpolate a value above 1024.'], ports=2))
    with open(os.path.join(rtl, 'aac_win_rom.v'), 'w') as f:
        f.write(verilog_rom(
            'aac_win_rom', 25, 4096, win_rom(),
            ['Left halves of the four windows of 4.6.11.3.2 as Q1.23:',
             'sine N=2048 at 0, KBD (alpha 4) N=2048 at 1024, sine N=256 at',
             '2048, KBD (alpha 6) N=256 at 2176. The right halves are these',
             'read backwards.']))
    tw = tw_rom()
    with open(os.path.join(rtl, 'aac_tw_rom.v'), 'w') as f:
        f.write(verilog_rom(
            'aac_tw_rom', 50, 1024,
            [((wr & 0x1FFFFFF) << 25) | (wi & 0x1FFFFFF) for wr, wi in tw],
            ['{cos, -sin} as two Q1.23 fields: exp(-i pi (n + 1/8) / M) for',
             'M = 1024 (n < 512, at 0) and M = 128 (n < 64, at 512), then',
             'exp(-2 pi i k / 512) for k < 256 at 576.']))


# ---------------------------------------------------------------------------
# Command line
# ---------------------------------------------------------------------------


def read_s16(path):
    data = open(path, 'rb').read()
    return [int.from_bytes(data[i:i + 2], 'little', signed=True)
            for i in range(0, len(data) - 1, 2)]


def frames_of(path, cov=None):
    return list(parse_adts(open(path, 'rb').read(), cov))


def decode_file(path, use_float=False):
    """Interleaved s16 PCM, every frame, the way ffmpeg writes it."""
    dec = FloatDecoder() if use_float else FixedDecoder()
    pcm = []
    for sfi, ch, els in frames_of(path):
        outs = dec.decode(sfi, ch, els)
        if use_float:
            outs = [[to_s16(v) for v in o] for o in outs]
        for n in range(1024):
            for o in outs:
                pcm.append(o[n])
    return pcm


def compare(a, b, skip=0):
    """(max abs error, SNR in dB of a against b, count) over a[skip:]."""
    n = min(len(a), len(b))
    err = sig = 0
    mx = 0
    for i in range(skip, n):
        d = a[i] - b[i]
        mx = max(mx, abs(d))
        err += d * d
        sig += b[i] * b[i]
    snr = 10 * math.log10(sig / err) if err else float('inf')
    return mx, snr, n - skip


def has_noise(els):
    return any(cb == NOISE_HCB for e in els for ics in e.ics
               for row in ics.sfb_cb for cb in row)


def compare_frames(path, ref, use_float=False):
    """Compares a decode of `path` with ffmpeg's PCM `ref`, frame by frame.

    ffmpeg drops the first frame of some streams (faac's), so the frame
    offset that lines the two up best is found first. Frames that use PNS
    are compared by energy only, since the noise is random by definition;
    the first frame is reported apart, since ffmpeg starts it from a sine
    left half where the standard says the first frame's own shape
    (4.6.11.3.2), and it is the encoder's delay in any case."""
    frames = frames_of(path)
    dec = FloatDecoder() if use_float else FixedDecoder()
    outs = []
    for fr in frames:
        o = dec.decode(*fr)
        if use_float:
            o = [[to_s16(v) for v in c] for c in o]
        outs.append(o)
    ch = frames[0][1]
    nref = len(ref) // (1024 * ch)
    best = None
    clean = [f for f in range(1, len(frames)) if not has_noise(frames[f][2])
             and not has_noise(frames[f - 1][2])]
    for off in range(0, 3):
        err = 0
        for f in range(1, min(nref, len(frames) - off)):
            if clean and (f + off) not in clean:
                continue
            for c in range(ch):
                for n in range(0, 1024, 7):
                    d = outs[f + off][c][n] - ref[(f * 1024 + n) * ch + c]
                    err += d * d
        if best is None or err < best[0]:
            best = (err, off)
    off = best[1]
    res = {'offset': off, 'exact_frames': 0, 'noise_frames': 0, 'max': 0,
           'err2': 0, 'sig2': 0, 'first_max': 0, 'noise_db': []}
    for f in range(min(nref, len(frames) - off)):
        o = outs[f + off]
        mx = e2 = s2 = 0
        for c in range(ch):
            for n in range(1024):
                r = ref[(f * 1024 + n) * ch + c]
                d = o[c][n] - r
                mx = max(mx, abs(d))
                e2 += d * d
                s2 += r * r
        if f + off == 0:
            res['first_max'] = mx
        elif has_noise(frames[f + off][2]) or has_noise(frames[f + off - 1][2]):
            res['noise_frames'] += 1
            ours = sum(v * v for c in o for v in c)
            res['noise_db'].append(10 * math.log10((ours + 1) / (s2 + 1)))
        else:
            res['exact_frames'] += 1
            res['max'] = max(res['max'], mx)
            res['err2'] += e2
            res['sig2'] += s2
    return res


def describe(res):
    snr = (10 * math.log10(res['sig2'] / res['err2']) if res['err2']
           else float('inf'))
    out = 'offset %d: %d frames max %d snr %.2f dB, first frame max %d' % (
        res['offset'], res['exact_frames'], res['max'], snr,
        res['first_max'])
    if res['noise_frames']:
        db = res['noise_db']
        out += '; %d PNS frames energy %+.2f..%+.2f dB' % (
            res['noise_frames'], min(db), max(db))
    return out


def main(argv):
    cmd = argv[1]
    if cmd == 'stats':
        cov = Coverage()
        frames_of(argv[2], cov)
        print(cov)
    elif cmd == 'decode':
        pcm = decode_file(argv[2], '--float' in argv)
        with open(argv[3], 'wb') as f:
            f.write(b''.join(int(v).to_bytes(2, 'little', signed=True)
                             for v in pcm))
    elif cmd == 'tables':
        write_tables(argv[2])
    elif cmd == 'compare':
        ref = read_s16(argv[3])
        print(describe(compare_frames(argv[2], ref, '--float' in argv)))
    else:
        raise SystemExit(__doc__)


if __name__ == '__main__':
    check_rate_tables()
    main(sys.argv)
