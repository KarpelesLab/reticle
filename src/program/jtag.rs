//! The IEEE 1149.1 test access port, and scans encoded onto an
//! [`Mpsse`].
//!
//! [`TapState`] is the sixteen-state controller of IEEE 1149.1-2013
//! figure 6-1, written as the one thing it actually is: a function from
//! (state, TMS) to state. Everything else — reaching a state, shifting an
//! instruction, shifting data, resting in `Run-Test/Idle` — is derived
//! from that function rather than from a table of hand-copied TMS
//! sequences, so there is nothing to mistype.
//!
//! # How a scan is encoded
//!
//! A scan of *N* bits ends by leaving the shift state, and the TAP leaves
//! it on the same clock as the last bit. The MPSSE cannot do both from a
//! data command, so a scan is always:
//!
//! 1. the TMS walk into `Shift-IR` or `Shift-DR`;
//! 2. the first *N-1* bits, as whole bytes then a remainder;
//! 3. the last bit, carried in bit 7 of a TMS command that also raises
//!    TMS, moving to `Exit1`;
//! 4. TMS `1` then `0`, through `Update` back to `Run-Test/Idle`.
//!
//! A captured scan therefore comes back in up to three pieces with two
//! different alignments, and [`unpack_capture`] is the one function that
//! knows how to put them together again. It is pure, and
//! [`Job::capture`] is how a caller reaches it.
//!
//! # Bit order
//!
//! Everything in this module is **least-significant bit first**, because
//! that is what JTAG is: bit 0 of a register is the first bit through
//! TDI, and the first bit out of TDO is bit 0 of what was captured. Data
//! that is natively most-significant-bit first — a Xilinx configuration
//! stream, for one — has to be turned round before it gets here, and
//! [`super::xilinx::reverse_bits`] is where that happens. Nothing in this
//! file reverses anything.

use super::ftdi::Mpsse;

/// A state of the IEEE 1149.1 TAP controller.
///
/// The variants are in the order of the standard's figure 6-1: the two
/// stable states, then the DR column, then the IR column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TapState {
    /// `Test-Logic-Reset`: reached from anywhere by five TMS-high clocks.
    TestLogicReset,
    /// `Run-Test/Idle`: the resting state between scans.
    RunTestIdle,
    /// `Select-DR-Scan`.
    SelectDr,
    /// `Capture-DR`.
    CaptureDr,
    /// `Shift-DR`.
    ShiftDr,
    /// `Exit1-DR`.
    Exit1Dr,
    /// `Pause-DR`.
    PauseDr,
    /// `Exit2-DR`.
    Exit2Dr,
    /// `Update-DR`.
    UpdateDr,
    /// `Select-IR-Scan`.
    SelectIr,
    /// `Capture-IR`.
    CaptureIr,
    /// `Shift-IR`.
    ShiftIr,
    /// `Exit1-IR`.
    Exit1Ir,
    /// `Pause-IR`.
    PauseIr,
    /// `Exit2-IR`.
    Exit2Ir,
    /// `Update-IR`.
    UpdateIr,
}

/// Every state, for iteration.
pub const TAP_STATES: [TapState; 16] = [
    TapState::TestLogicReset,
    TapState::RunTestIdle,
    TapState::SelectDr,
    TapState::CaptureDr,
    TapState::ShiftDr,
    TapState::Exit1Dr,
    TapState::PauseDr,
    TapState::Exit2Dr,
    TapState::UpdateDr,
    TapState::SelectIr,
    TapState::CaptureIr,
    TapState::ShiftIr,
    TapState::Exit1Ir,
    TapState::PauseIr,
    TapState::Exit2Ir,
    TapState::UpdateIr,
];

impl TapState {
    /// The state one TCK later, given the TMS level on that clock. This
    /// is IEEE 1149.1-2013 figure 6-1 and nothing else in this module
    /// duplicates it.
    #[must_use]
    pub fn step(self, tms: bool) -> TapState {
        use TapState::*;
        match (self, tms) {
            (TestLogicReset, true) => TestLogicReset,
            (TestLogicReset, false) => RunTestIdle,
            (RunTestIdle, true) => SelectDr,
            (RunTestIdle, false) => RunTestIdle,

            (SelectDr, true) => SelectIr,
            (SelectDr, false) => CaptureDr,
            (CaptureDr, true) => Exit1Dr,
            (CaptureDr, false) => ShiftDr,
            (ShiftDr, true) => Exit1Dr,
            (ShiftDr, false) => ShiftDr,
            (Exit1Dr, true) => UpdateDr,
            (Exit1Dr, false) => PauseDr,
            (PauseDr, true) => Exit2Dr,
            (PauseDr, false) => PauseDr,
            (Exit2Dr, true) => UpdateDr,
            (Exit2Dr, false) => ShiftDr,
            (UpdateDr, true) => SelectDr,
            (UpdateDr, false) => RunTestIdle,

            (SelectIr, true) => TestLogicReset,
            (SelectIr, false) => CaptureIr,
            (CaptureIr, true) => Exit1Ir,
            (CaptureIr, false) => ShiftIr,
            (ShiftIr, true) => Exit1Ir,
            (ShiftIr, false) => ShiftIr,
            (Exit1Ir, true) => UpdateIr,
            (Exit1Ir, false) => PauseIr,
            (PauseIr, true) => Exit2Ir,
            (PauseIr, false) => PauseIr,
            (Exit2Ir, true) => UpdateIr,
            (Exit2Ir, false) => ShiftIr,
            (UpdateIr, true) => SelectDr,
            (UpdateIr, false) => RunTestIdle,
        }
    }

    /// The shortest TMS sequence from here to `target`, as bits in the
    /// order they are clocked (bit 0 first) and how many there are.
    ///
    /// Breadth-first over [`step`](Self::step), so it cannot disagree
    /// with the state machine. The graph is small and the longest
    /// shortest path is seven clocks, so the bits always fit in a byte —
    /// which is exactly one MPSSE TMS command.
    #[must_use]
    pub fn path_to(self, target: TapState) -> (u8, u8) {
        if self == target {
            return (0, 0);
        }
        // (state, bits so far, length so far), visited by state.
        let mut seen = [false; 16];
        let mut queue = std::collections::VecDeque::new();
        seen[self as usize] = true;
        queue.push_back((self, 0u8, 0u8));
        while let Some((state, bits, len)) = queue.pop_front() {
            for tms in [false, true] {
                let next = state.step(tms);
                if seen[next as usize] {
                    continue;
                }
                let bits = if tms { bits | (1 << len) } else { bits };
                let len = len + 1;
                if next == target {
                    return (bits, len);
                }
                seen[next as usize] = true;
                queue.push_back((next, bits, len));
            }
        }
        unreachable!("every TAP state reaches every other")
    }

    /// True in `Shift-DR` or `Shift-IR`, where TDI and TDO carry data.
    #[must_use]
    pub fn is_shift(self) -> bool {
        matches!(self, TapState::ShiftDr | TapState::ShiftIr)
    }
}

/// Where one captured scan's bytes sit in a reply, and how wide it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Capture {
    /// Offset of the scan's first byte in the stripped reply.
    offset: usize,
    /// How many bits the scan shifted.
    bits: usize,
}

/// A buffer of MPSSE commands that performs a sequence of JTAG
/// operations, together with what its reply means.
///
/// Built by [`Scan`], sent by [`super::usb`], and decoded back here by
/// [`Job::capture`] so the decoding never drifts from the encoding.
#[derive(Clone, Debug)]
pub struct Job {
    commands: Vec<u8>,
    read_len: usize,
    captures: Vec<Capture>,
}

impl Job {
    /// The bytes to write to the device's bulk OUT endpoint.
    #[must_use]
    pub fn commands(&self) -> &[u8] {
        &self.commands
    }

    /// How many bytes of captured TDO to expect back, after the
    /// modem-status bytes have been stripped.
    #[must_use]
    pub fn read_len(&self) -> usize {
        self.read_len
    }

    /// How many captured scans this job contains.
    #[must_use]
    pub fn capture_count(&self) -> usize {
        self.captures.len()
    }

    /// The `index`-th captured scan, as bits packed least-significant
    /// first: bit *i* of the register is bit `i % 8` of byte `i / 8`.
    ///
    /// `reply` is the whole job's reply with the modem-status bytes
    /// already removed.
    ///
    /// # Errors
    ///
    /// [`JobError::NoSuchCapture`] for an index this job does not have,
    /// and [`JobError::ShortReply`] when the device returned fewer bytes
    /// than the commands asked for — which is what a timeout looks like
    /// from here.
    pub fn capture(&self, index: usize, reply: &[u8]) -> Result<Vec<u8>, JobError> {
        let capture = *self
            .captures
            .get(index)
            .ok_or(JobError::NoSuchCapture(index))?;
        let len = capture_len(capture.bits);
        let end = capture.offset + len;
        if reply.len() < end {
            return Err(JobError::ShortReply {
                wanted: self.read_len,
                got: reply.len(),
            });
        }
        Ok(unpack_capture(&reply[capture.offset..end], capture.bits))
    }

    /// The `index`-th captured scan as a 32-bit register whose **bit 0
    /// came out of TDO first**, which is how IEEE 1149.1 defines
    /// `IDCODE`.
    ///
    /// A register that shifts out most-significant bit first — the
    /// Xilinx configuration registers read through `CFG_OUT`, for one —
    /// is *not* this, and [`super::xilinx::word_msb_first`] is what
    /// reads those.
    ///
    /// # Errors
    ///
    /// As [`capture`](Self::capture), plus [`JobError::WrongWidth`] when
    /// the scan was not 32 bits wide.
    pub fn capture_u32(&self, index: usize, reply: &[u8]) -> Result<u32, JobError> {
        let capture = *self
            .captures
            .get(index)
            .ok_or(JobError::NoSuchCapture(index))?;
        if capture.bits != 32 {
            return Err(JobError::WrongWidth {
                wanted: 32,
                got: capture.bits,
            });
        }
        let bytes = self.capture(index, reply)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

/// Why a reply could not be read back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobError {
    /// The job has no capture with this index.
    NoSuchCapture(usize),
    /// The device returned fewer bytes than the commands asked for.
    ShortReply {
        /// Bytes the commands should have produced.
        wanted: usize,
        /// Bytes that arrived.
        got: usize,
    },
    /// A capture was read as a width it does not have.
    WrongWidth {
        /// The width asked for.
        wanted: usize,
        /// The width the scan actually shifted.
        got: usize,
    },
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobError::NoSuchCapture(i) => write!(f, "this job has no capture {i}"),
            JobError::ShortReply { wanted, got } => write!(
                f,
                "the adapter returned {got} byte(s) of TDO, not the {wanted} the commands ask for"
            ),
            JobError::WrongWidth { wanted, got } => {
                write!(f, "that scan shifted {got} bits, not {wanted}")
            }
        }
    }
}

impl std::error::Error for JobError {}

/// How many reply bytes a captured scan of `bits` bits produces.
///
/// Whole bytes for the first `bits - 1`, one more byte if a remainder is
/// left over, and one for the final bit that leaves with TMS.
#[must_use]
pub fn capture_len(bits: usize) -> usize {
    assert!(bits > 0, "a scan shifts at least one bit");
    let head = bits - 1;
    head / 8 + usize::from(!head.is_multiple_of(8)) + 1
}

/// Reassembles one captured scan out of the pieces the MPSSE returns.
///
/// The three pieces do not share an alignment, which is the whole
/// difficulty:
///
/// - the byte command's bytes are already LSB-first, bit *i* in bit
///   `i % 8`;
/// - the bit command's single byte is **left-aligned**: the MPSSE shifts
///   each captured bit in at bit 7 and moves the byte right, so after `r`
///   bits the first one is at bit `8 - r`;
/// - the final TMS command captured exactly one bit, which by the same
///   rule sits in bit 7.
///
/// The result is `bits` bits packed least-significant first.
///
/// # Panics
///
/// When `chunk` is shorter than [`capture_len`] of `bits`, or `bits` is
/// zero. Both are caller bugs; [`Job::capture`] checks the length first
/// and reports [`JobError::ShortReply`] instead.
#[must_use]
pub fn unpack_capture(chunk: &[u8], bits: usize) -> Vec<u8> {
    assert!(bits > 0, "a scan shifts at least one bit");
    assert!(chunk.len() >= capture_len(bits), "capture chunk too short");
    let head = bits - 1;
    let whole = head / 8;
    let rem = head % 8;

    let mut out = vec![0u8; bits.div_ceil(8)];
    for i in 0..whole * 8 {
        if chunk[i / 8] >> (i % 8) & 1 == 1 {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    let mut next = whole;
    if rem != 0 {
        let byte = chunk[next] >> (8 - rem);
        for k in 0..rem {
            let i = whole * 8 + k;
            if byte >> k & 1 == 1 {
                out[i / 8] |= 1 << (i % 8);
            }
        }
        next += 1;
    }
    if chunk[next] >> 7 & 1 == 1 {
        let i = bits - 1;
        out[i / 8] |= 1 << (i % 8);
    }
    out
}

/// Builds a [`Job`]: TAP moves and scans, in order.
///
/// The builder tracks the TAP state, so `shift_ir` after `shift_dr`
/// walks the right path without the caller counting clocks. It starts in
/// `Test-Logic-Reset` only after [`reset`](Self::reset); before that the
/// state is assumed to be `Test-Logic-Reset`, which is where five TMS
/// clocks put any TAP and where every sequence in this crate begins.
#[derive(Clone, Debug)]
pub struct Scan {
    mpsse: Mpsse,
    state: TapState,
    captures: Vec<Capture>,
}

impl Default for Scan {
    fn default() -> Scan {
        Scan::new()
    }
}

impl Scan {
    /// A builder whose TAP is assumed to be in `Test-Logic-Reset`.
    #[must_use]
    pub fn new() -> Scan {
        Scan {
            mpsse: Mpsse::new(),
            state: TapState::TestLogicReset,
            captures: Vec::new(),
        }
    }

    /// A builder that starts from an [`Mpsse`] already carrying setup
    /// commands (the clock divisor and the pin directions), with the TAP
    /// in `Test-Logic-Reset`.
    #[must_use]
    pub fn from_mpsse(mpsse: Mpsse) -> Scan {
        Scan {
            mpsse,
            state: TapState::TestLogicReset,
            captures: Vec::new(),
        }
    }

    /// Where the TAP will be once the commands so far have run.
    #[must_use]
    pub fn state(&self) -> TapState {
        self.state
    }

    /// Forces the TAP to `Test-Logic-Reset` with five TMS-high clocks,
    /// which works from any state including an unknown one, then rests in
    /// `Run-Test/Idle`.
    pub fn reset(&mut self) {
        self.mpsse.write_tms(0b11111, 5, false);
        self.state = TapState::TestLogicReset;
        self.goto(TapState::RunTestIdle);
    }

    /// Walks the TAP to `state` by the shortest TMS path.
    ///
    /// One MPSSE TMS command carries at most seven TMS bits (bit 7 of
    /// its data byte belongs to TDI), and two of the sixteen-state
    /// machine's shortest paths are eight clocks long, so a walk may
    /// take two commands. Splitting is safe: TMS is clocked one bit at
    /// a time and the TAP does not care where a USB packet ends.
    pub fn goto(&mut self, state: TapState) {
        let (bits, len) = self.state.path_to(state);
        let mut sent = 0;
        while sent < len {
            let n = (len - sent).min(7);
            let chunk = (bits >> sent) & ((1u8 << n) - 1);
            self.mpsse.write_tms(chunk, n, false);
            sent += n;
        }
        self.state = state;
    }

    /// Holds the TAP in `Run-Test/Idle` for `cycles` clocks of TCK.
    ///
    /// Data commands do not touch TMS, so once the TAP is in
    /// `Run-Test/Idle` (where TMS is low) whole bytes of dummy TDI are
    /// the cheapest way to clock it: 8 TCK per command byte instead of
    /// 8 TCK per three. That matters after `JPROGRAM`, which wants tens
    /// of thousands of clocks.
    pub fn idle(&mut self, cycles: usize) {
        if cycles == 0 {
            return;
        }
        self.goto(TapState::RunTestIdle);
        let bytes = cycles / 8;
        if bytes != 0 {
            let zeros = vec![0u8; bytes];
            self.mpsse.write_bytes(&zeros);
        }
        let rest = cycles % 8;
        if rest != 0 {
            self.mpsse.write_bits(0, u8::try_from(rest).unwrap_or(1));
        }
    }

    /// Shifts `bits` bits of `data` (packed least-significant first)
    /// through `Shift-IR` and returns to `Run-Test/Idle`.
    ///
    /// # Panics
    ///
    /// When `bits` is zero or `data` is too short for it.
    pub fn shift_ir(&mut self, data: &[u8], bits: usize) {
        self.shift(TapState::ShiftIr, data, bits, false);
    }

    /// Shifts `bits` bits of `data` through `Shift-DR` and returns to
    /// `Run-Test/Idle`, capturing nothing.
    ///
    /// # Panics
    ///
    /// When `bits` is zero or `data` is too short for it.
    pub fn shift_dr(&mut self, data: &[u8], bits: usize) {
        self.shift(TapState::ShiftDr, data, bits, false);
    }

    /// Shifts `bits` zero bits through `Shift-DR` while capturing TDO,
    /// and returns to `Run-Test/Idle`. The capture's index is its
    /// position among this job's captures, counting from zero.
    ///
    /// # Panics
    ///
    /// When `bits` is zero.
    pub fn read_dr(&mut self, bits: usize) -> usize {
        let index = self.captures.len();
        let zeros = vec![0u8; bits.div_ceil(8)];
        self.shift(TapState::ShiftDr, &zeros, bits, true);
        index
    }

    /// Finishes the job, asking the device to flush any capture.
    #[must_use]
    pub fn finish(mut self) -> Job {
        if self.mpsse.read_len() != 0 {
            self.mpsse.send_immediate();
        }
        Job {
            read_len: self.mpsse.read_len(),
            commands: self.mpsse.into_commands(),
            captures: self.captures,
        }
    }

    /// The shared body of every scan: walk in, shift `bits - 1` bits,
    /// send the last one out with TMS, walk back to `Run-Test/Idle`.
    fn shift(&mut self, shift_state: TapState, data: &[u8], bits: usize, capture: bool) {
        assert!(bits > 0, "a scan shifts at least one bit");
        assert!(
            data.len() >= bits.div_ceil(8),
            "not enough data for the scan"
        );
        if capture {
            self.captures.push(Capture {
                offset: self.mpsse.read_len(),
                bits,
            });
        }
        self.goto(shift_state);

        let head = bits - 1;
        let whole = head / 8;
        let rem = head % 8;
        if whole != 0 {
            if capture {
                self.mpsse.exchange_bytes(&data[..whole]);
            } else {
                self.mpsse.write_bytes(&data[..whole]);
            }
        }
        if rem != 0 {
            let byte = data[whole] & ((1 << rem) - 1);
            let count = u8::try_from(rem).unwrap_or(1);
            if capture {
                self.mpsse.exchange_bits(byte, count);
            } else {
                self.mpsse.write_bits(byte, count);
            }
        }

        // The last bit leaves on the clock that raises TMS, so it rides
        // in bit 7 of a TMS command rather than a data one.
        let last = data[head / 8] >> (head % 8) & 1 == 1;
        if capture {
            self.mpsse.exchange_tms(0b1, 1, last);
        } else {
            self.mpsse.write_tms(0b1, 1, last);
        }
        self.state = self.state.step(true);

        // Exit1 -> Update -> Run-Test/Idle.
        self.goto(TapState::RunTestIdle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Five TMS-high clocks reach `Test-Logic-Reset` from every state,
    /// which is the property the whole reset path relies on.
    #[test]
    fn five_tms_clocks_reset_from_anywhere() {
        for start in TAP_STATES {
            let mut s = start;
            for _ in 0..5 {
                s = s.step(true);
            }
            assert_eq!(s, TapState::TestLogicReset, "from {start:?}");
        }
    }

    /// Every state reaches every other, and the path the search returns
    /// really does walk there. Eight clocks is the longest shortest path
    /// in the machine (`Capture-DR` to `Exit2-IR` and its mirror), which
    /// is why [`Scan::goto`] splits at seven.
    #[test]
    fn the_paths_are_short_and_correct() {
        for from in TAP_STATES {
            for to in TAP_STATES {
                let (bits, len) = from.path_to(to);
                assert!(len <= 8, "{from:?} -> {to:?} takes {len} clocks");
                let mut s = from;
                for i in 0..len {
                    s = s.step(bits >> i & 1 == 1);
                }
                assert_eq!(s, to, "{from:?} -> {to:?} with {bits:#b}/{len}");
            }
        }
    }

    /// The well-known walks, spelled out so a refactor of the search
    /// cannot quietly produce a different (but still valid) route.
    #[test]
    fn the_well_known_walks_are_the_expected_ones() {
        // Run-Test/Idle -> Shift-DR is TMS 1, 0, 0.
        assert_eq!(TapState::RunTestIdle.path_to(TapState::ShiftDr), (0b001, 3));
        // Run-Test/Idle -> Shift-IR is TMS 1, 1, 0, 0.
        assert_eq!(
            TapState::RunTestIdle.path_to(TapState::ShiftIr),
            (0b0011, 4)
        );
        // Exit1 -> Update -> Run-Test/Idle is TMS 1, 0.
        assert_eq!(TapState::Exit1Dr.path_to(TapState::RunTestIdle), (0b01, 2));
        assert_eq!(TapState::Exit1Ir.path_to(TapState::RunTestIdle), (0b01, 2));
        // Test-Logic-Reset -> Run-Test/Idle is one low clock.
        assert_eq!(
            TapState::TestLogicReset.path_to(TapState::RunTestIdle),
            (0b0, 1)
        );
        assert_eq!(TapState::ShiftDr.path_to(TapState::ShiftDr), (0, 0));
    }

    /// A capture of `n` bits takes the number of reply bytes the
    /// encoding implies, for every width the crate uses and a few more.
    #[test]
    fn a_capture_is_as_long_as_the_encoding_says() {
        assert_eq!(capture_len(1), 1); // last bit only
        assert_eq!(capture_len(6), 2); // 5 bits + last
        assert_eq!(capture_len(8), 2); // 7 bits + last
        assert_eq!(capture_len(9), 2); // 1 byte + last
        assert_eq!(capture_len(32), 5); // 3 bytes + 7 bits + last
        assert_eq!(capture_len(33), 5); // 4 bytes + last
    }

    /// Unpacking puts the three differently aligned pieces back in
    /// order. The reply here is hand-built from the alignment rules, so
    /// this is the test that would catch an off-by-one in either.
    #[test]
    fn unpacking_reassembles_the_three_pieces() {
        // A 32-bit scan: 3 whole bytes, then 7 bits left-aligned, then
        // the last bit in bit 7.
        //
        // Value 0x8765_4321, LSB first: bytes 21 43 65 87.
        // bits 0..23  -> 0x21, 0x43, 0x65
        // bits 24..30 -> low 7 bits of 0x87 = 0b000_0111, left-aligned
        //                in a byte: 0b0000111 << 1 = 0x0E
        // bit 31      -> 1, so 0x80.
        let reply = [0x21, 0x43, 0x65, 0x0E, 0x80];
        assert_eq!(unpack_capture(&reply, 32), [0x21, 0x43, 0x65, 0x87]);

        // One bit only: the last-bit byte is all there is.
        assert_eq!(unpack_capture(&[0x80], 1), [0x01]);
        assert_eq!(unpack_capture(&[0x00], 1), [0x00]);

        // Six bits, 0b101101: 5 bits 0b01101 left-aligned = 0b01101_000,
        // then the sixth bit (1) in bit 7.
        assert_eq!(unpack_capture(&[0b0110_1000, 0x80], 6), [0b0010_1101]);
    }

    /// An `IDCODE` read decodes to the value the part shifted out, with
    /// its LSB first. Feeding the job a synthetic reply is the whole
    /// hardware-free test of the read path.
    #[test]
    fn a_thirty_two_bit_read_decodes_lsb_first() {
        let mut scan = Scan::new();
        scan.reset();
        let idx = scan.read_dr(32);
        let job = scan.finish();
        assert_eq!(idx, 0);
        assert_eq!(job.capture_count(), 1);
        assert_eq!(job.read_len(), 5);

        // 0x0362D093 shifted out LSB first.
        let value: u32 = 0x0362_D093;
        let lsb = value.to_le_bytes();
        let reply = [
            lsb[0],
            lsb[1],
            lsb[2],
            (lsb[3] & 0x7F) << 1,
            if lsb[3] & 0x80 != 0 { 0x80 } else { 0 },
        ];
        assert_eq!(job.capture_u32(0, &reply).unwrap(), value);
        assert_eq!(job.capture(0, &reply).unwrap(), lsb);
    }

    /// A short reply is reported, not silently padded: a timeout on the
    /// bulk read must not look like an `IDCODE` of zero.
    #[test]
    fn a_short_reply_is_an_error() {
        let mut scan = Scan::new();
        let _ = scan.read_dr(32);
        let job = scan.finish();
        assert_eq!(
            job.capture_u32(0, &[0, 0, 0]),
            Err(JobError::ShortReply { wanted: 5, got: 3 })
        );
        assert_eq!(job.capture(1, &[0; 5]), Err(JobError::NoSuchCapture(1)));

        let mut scan = Scan::new();
        let _ = scan.read_dr(6);
        let job = scan.finish();
        assert!(matches!(
            job.capture_u32(0, &[0; 2]),
            Err(JobError::WrongWidth { wanted: 32, got: 6 })
        ));
    }

    /// Two captures in one job keep their own offsets into the reply.
    #[test]
    fn captures_keep_their_own_offsets() {
        let mut scan = Scan::new();
        scan.reset();
        let a = scan.read_dr(32);
        scan.idle(8);
        let b = scan.read_dr(8);
        let job = scan.finish();
        assert_eq!((a, b), (0, 1));
        assert_eq!(job.read_len(), 5 + 2);

        let mut reply = vec![0u8; 7];
        reply[0] = 0x11;
        reply[5] = 0b0101_0100; // 7 bits, left-aligned: 0b1010101 -> wait
        reply[6] = 0x80;
        let first = job.capture(a, &reply).unwrap();
        assert_eq!(first[0], 0x11);
        let second = job.capture(b, &reply).unwrap();
        assert_eq!(second.len(), 1);
        // 7 captured bits 0b0101010 (LSB first) plus a final 1 at bit 7.
        assert_eq!(second[0], 0b1010_1010);
    }

    /// A write-only scan produces no capture and asks for no reply, so a
    /// two-megabyte configuration shift never waits on the IN endpoint.
    #[test]
    fn a_write_only_scan_reads_nothing() {
        let mut scan = Scan::new();
        scan.reset();
        scan.shift_dr(&[0xAA; 1024], 8192);
        let job = scan.finish();
        assert_eq!(job.read_len(), 0);
        assert_eq!(job.capture_count(), 0);
        // No CMD_SEND_IMMEDIATE when there is nothing to flush.
        assert_ne!(
            job.commands().last().copied(),
            Some(super::super::ftdi::CMD_SEND_IMMEDIATE)
        );
    }

    /// `idle` clocks exactly the number of TCK asked for, using byte
    /// commands for the bulk of them.
    #[test]
    fn idle_clocks_the_requested_number_of_times() {
        let mut scan = Scan::new();
        scan.reset();
        let before = scan.mpsse.len();
        scan.idle(8 * 100 + 3);
        let added = &scan.mpsse.commands()[before..];
        // One byte command (3 bytes of header + 100 of data) and one bit
        // command (3 bytes).
        assert_eq!(added.len(), 3 + 100 + 3);
        assert_eq!(added[0], super::super::ftdi::CMD_BYTES_OUT);
        assert_eq!(added[103], super::super::ftdi::CMD_BITS_OUT);
        assert_eq!(added[104], 2); // three bits, minus one
        assert_eq!(scan.state(), TapState::RunTestIdle);

        let mut scan = Scan::new();
        let before = scan.mpsse.len();
        scan.idle(0);
        assert_eq!(scan.mpsse.len(), before);
    }

    /// After any scan the builder is back in `Run-Test/Idle`, which is
    /// what lets the next one start from a known place.
    #[test]
    fn every_scan_ends_in_run_test_idle() {
        let mut scan = Scan::new();
        scan.reset();
        assert_eq!(scan.state(), TapState::RunTestIdle);
        scan.shift_ir(&[0x09], 6);
        assert_eq!(scan.state(), TapState::RunTestIdle);
        let _ = scan.read_dr(32);
        assert_eq!(scan.state(), TapState::RunTestIdle);
        scan.shift_dr(&[0xFF, 0x00], 16);
        assert_eq!(scan.state(), TapState::RunTestIdle);
    }
}
