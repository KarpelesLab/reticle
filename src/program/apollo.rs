//! The Apollo debugger's request set, and a [`jtag::Plan`] compiled into
//! it.
//!
//! Apollo is the firmware on the debug microcontroller of a Great Scott
//! Gadgets Cynthion. Everything in this file is written from
//! `docs/apollo-protocol.md`, which is Reticle's own description of the
//! wire protocol; that document was written first, on purpose, and says
//! for every number where it came from and how confident this project is
//! of it. Apollo itself is BSD-3-Clause and copying it would have been
//! allowed — it was not copied, for the same reason Reticle's VHDL
//! library carries its own declarations rather than IEEE's text.
//!
//! Like [`super::ftdi`], nothing here performs I/O. A [`Program`] is a
//! list of control transfers waiting to be made; [`super::usb`] makes
//! them.
//!
//! # How this differs from the FTDI transport
//!
//! An MPSSE is a shift engine: the host encodes TMS bits and the TAP
//! walk is the host's business. Apollo is the other shape — the
//! microcontroller *owns* a TAP controller, and the host names states
//! and bit counts. There is no way to send a TMS sequence at all.
//!
//! So the two transports cannot share an encoding, and the thing they do
//! share is one level up: [`jtag::Plan`], a list of named operations.
//! [`jtag::Scan::apply`] turns a plan into MPSSE bytes and [`compile`]
//! turns the same plan into the requests below. The TAP state machine,
//! the state names and their order are [`jtag`]'s in both cases.

use super::jtag::{self, Op, TapState};

// ---------------------------------------------------------------------
// Identities
// ---------------------------------------------------------------------

/// The USB vendor identifier a Cynthion uses, in either mode. It is
/// Openmoko's, administered by pid.codes.
pub const VENDOR_ID: u16 = 0x1D50;

/// The product identifier of the analyzer gateware — the board with the
/// FPGA driving the USB port.
pub const GATEWARE_PRODUCT_ID: u16 = 0x615B;

/// The product identifier of the Apollo debugger — the board with the
/// microcontroller driving the USB port.
pub const DEBUGGER_PRODUCT_ID: u16 = 0x615C;

/// `bInterfaceClass` of both the gateware's interfaces and the stub's.
pub const VENDOR_SPECIFIC_CLASS: u8 = 0xFF;

/// `bInterfaceSubClass` of the Apollo stub interface. The gateware's own
/// interfaces are vendor specific too, so the subclass is what tells
/// them apart, and the stub's number is not fixed by anything.
pub const STUB_SUBCLASS: u8 = 0x00;

// ---------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------

/// Host to device, vendor, recipient device: every request Apollo itself
/// answers that carries no reply.
pub const REQ_TYPE_OUT: u8 = 0x40;

/// Device to host, vendor, recipient device: every request Apollo itself
/// answers with data.
pub const REQ_TYPE_IN: u8 = 0xC0;

/// Host to device, vendor, **recipient interface**. Exactly one request
/// uses this — the handover in [`REQUEST_ADVERTISEMENT_STOP`] — because
/// exactly one request is answered by the FPGA's gateware rather than by
/// the microcontroller.
pub const REQ_TYPE_OUT_INTERFACE: u8 = 0x41;

// ---------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------

/// Tell the gateware to stop advertising for the USB port, which makes
/// the microcontroller take it. Sent to the stub interface; the device
/// then re-enumerates as [`DEBUGGER_PRODUCT_ID`].
pub const REQUEST_ADVERTISEMENT_STOP: u8 = 0xF0;

/// Read a NUL-terminated string naming the firmware. It contains
/// `Apollo`.
pub const REQUEST_GET_ID: u8 = 0xA0;
/// Read a NUL-terminated string: the firmware version.
pub const REQUEST_GET_FIRMWARE_VERSION: u8 = 0xA2;
/// Read two bytes, major then minor, of the USB API version.
pub const REQUEST_GET_USB_API_VERSION: u8 = 0xA3;
/// Let the FPGA have the USB port back, undoing
/// [`REQUEST_ADVERTISEMENT_STOP`].
pub const REQUEST_ALLOW_FPGA_TAKEOVER_USB: u8 = 0xC2;

/// Zero the buffer of bits waiting to go out on TDI.
pub const REQUEST_JTAG_CLEAR_OUT_BUFFER: u8 = 0xB0;
/// Fill that buffer from the data stage.
pub const REQUEST_JTAG_SET_OUT_BUFFER: u8 = 0xB1;
/// Read back the bits the last scan captured on TDO.
pub const REQUEST_JTAG_GET_IN_BUFFER: u8 = 0xB2;
/// Shift `wValue` bits, with `wIndex` carrying [`FLAG_ADVANCE_STATE`]
/// and [`FLAG_FORCE_BITBANG`].
pub const REQUEST_JTAG_SCAN: u8 = 0xB3;
/// Clock TCK `wValue` times without leaving the current state.
pub const REQUEST_JTAG_RUN_CLOCK: u8 = 0xB4;
/// Walk the TAP to the state numbered by `wValue`.
pub const REQUEST_JTAG_GO_TO_STATE: u8 = 0xB5;
/// Read one byte: the TAP state the firmware believes it is in.
pub const REQUEST_JTAG_GET_STATE: u8 = 0xB6;
/// Read eight bytes of capability; see [`Capability`].
pub const REQUEST_JTAG_GET_INFO: u8 = 0xB8;
/// Release the JTAG pins.
pub const REQUEST_JTAG_STOP: u8 = 0xBE;
/// Take the JTAG pins.
pub const REQUEST_JTAG_START: u8 = 0xBF;

/// `wIndex` bit 0 of a scan: raise TMS with the last bit, so the scan
/// ends in `Exit1-DR` or `Exit1-IR` rather than staying in the shift
/// state.
pub const FLAG_ADVANCE_STATE: u16 = 1 << 0;

/// `wIndex` bit 1 of a scan: shift it one bit at a time rather than with
/// the hardware engine.
pub const FLAG_FORCE_BITBANG: u16 = 1 << 1;

/// Quirk bit 0: the shift engine moves whole bytes most-significant bit
/// first, so the host bit-reverses every whole byte in both directions.
pub const QUIRK_FLIP_BITS_IN_WHOLE_BYTES: u32 = 1 << 0;

/// Quirk bit 1: every scan should set [`FLAG_FORCE_BITBANG`].
pub const QUIRK_ALWAYS_BITBANG: u32 = 1 << 1;

/// What to assume when [`REQUEST_JTAG_GET_INFO`] stalls, because the
/// firmware predates it: 2048 bits, which is the 256-byte buffer the
/// firmware has each way.
pub const DEFAULT_MAX_SCAN_BITS: u32 = 2048;

// ---------------------------------------------------------------------
// TAP state numbers
// ---------------------------------------------------------------------

/// The number Apollo gives a TAP state in
/// [`REQUEST_JTAG_GO_TO_STATE`] and [`REQUEST_JTAG_GET_STATE`].
///
/// Apollo numbers the sixteen states of IEEE 1149.1 figure 6-1 in the
/// order the standard draws them, which is the order
/// [`jtag::TAP_STATES`] is already in. That coincidence is *used* rather
/// than copied — the number is the state's position in Reticle's own
/// table — and a test pins all sixteen pairs, because "the same order"
/// is the kind of claim that stays true until somebody reorders an enum.
#[must_use]
pub fn state_number(state: TapState) -> u16 {
    let index = jtag::TAP_STATES
        .iter()
        .position(|&s| s == state)
        .expect("every TAP state is in TAP_STATES");
    u16::try_from(index).expect("sixteen states fit in a u16")
}

/// The state a number names, or `None` for a byte outside 0..16.
#[must_use]
pub fn state_from_number(number: u8) -> Option<TapState> {
    jtag::TAP_STATES.get(usize::from(number)).copied()
}

// ---------------------------------------------------------------------
// Capability
// ---------------------------------------------------------------------

/// What [`REQUEST_JTAG_GET_INFO`] says the firmware can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capability {
    /// The largest number of bits one scan may shift.
    pub max_scan_bits: u32,
    /// The quirk flags, of which this crate understands
    /// [`QUIRK_FLIP_BITS_IN_WHOLE_BYTES`] and [`QUIRK_ALWAYS_BITBANG`].
    pub quirks: u32,
}

impl Default for Capability {
    fn default() -> Capability {
        Capability {
            max_scan_bits: DEFAULT_MAX_SCAN_BITS,
            quirks: 0,
        }
    }
}

impl Capability {
    /// Decodes the eight-byte reply: two little-endian 32-bit fields,
    /// the scan limit then the quirks.
    ///
    /// Returns `None` for a short reply rather than guessing, since a
    /// wrong limit is a scan the firmware silently truncates.
    #[must_use]
    pub fn from_reply(bytes: &[u8]) -> Option<Capability> {
        if bytes.len() < 8 {
            return None;
        }
        Some(Capability {
            max_scan_bits: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            quirks: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        })
    }

    /// Whether whole bytes have to be bit-reversed in both directions.
    #[must_use]
    pub fn flips_whole_bytes(self) -> bool {
        self.quirks & QUIRK_FLIP_BITS_IN_WHOLE_BYTES != 0
    }

    /// Whether every scan should ask to be bit-banged.
    #[must_use]
    pub fn always_bitbangs(self) -> bool {
        self.quirks & QUIRK_ALWAYS_BITBANG != 0
    }

    /// The `wIndex` of a scan, given whether it is the one that leaves
    /// the shift state.
    #[must_use]
    pub fn scan_flags(self, advance: bool) -> u16 {
        let mut flags = 0;
        if advance {
            flags |= FLAG_ADVANCE_STATE;
        }
        if self.always_bitbangs() {
            flags |= FLAG_FORCE_BITBANG;
        }
        flags
    }

    /// How many bits one scan of a long register may carry.
    ///
    /// Three limits meet here, and the smallest wins: what the firmware
    /// said, what fits in a 16-bit `wValue`, and a whole number of bytes
    /// — because a chunk that ends mid-byte would make the pieces of a
    /// split capture need bit-level reassembly, and there is nothing to
    /// gain from that. The floor is 8 bits, so a firmware reporting
    /// nonsense produces slow progress rather than a divide by zero.
    #[must_use]
    pub fn chunk_bits(self) -> usize {
        let limit = usize::try_from(self.max_scan_bits.min(u32::from(u16::MAX)))
            .expect("a u16 fits in a usize");
        (limit & !7).max(8)
    }
}

// ---------------------------------------------------------------------
// The whole-byte bit flip
// ---------------------------------------------------------------------

/// Bit-reverses every **whole** byte of a scan of `bits` bits, leaving a
/// trailing partial byte alone.
///
/// This is [`QUIRK_FLIP_BITS_IN_WHOLE_BYTES`]. It is its own inverse, so
/// the same function serves the bits going out and the bits coming back.
#[must_use]
pub fn flip_whole_bytes(data: &[u8], bits: usize) -> Vec<u8> {
    let mut out = data.to_vec();
    for byte in out.iter_mut().take(bits / 8) {
        *byte = byte.reverse_bits();
    }
    out
}

// ---------------------------------------------------------------------
// Programs
// ---------------------------------------------------------------------

/// One control transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Host to device, no data stage.
    Command {
        /// `bRequest`.
        request: u8,
        /// `wValue`.
        value: u16,
        /// `wIndex`.
        index: u16,
    },
    /// Host to device, carrying data.
    Write {
        /// `bRequest`.
        request: u8,
        /// `wValue`.
        value: u16,
        /// `wIndex`.
        index: u16,
        /// The data stage.
        data: Vec<u8>,
    },
    /// Device to host.
    Read {
        /// `bRequest`.
        request: u8,
        /// `wValue`.
        value: u16,
        /// `wIndex`.
        index: u16,
        /// How many bytes to ask for.
        len: usize,
        /// Which capture these bytes belong to, and how many of their
        /// bits are part of it. A capture split across several scans
        /// contributes one `Read` each, in order, and the pieces
        /// concatenate as bytes — [`Capability::chunk_bits`] is a whole
        /// number of bytes so that they can.
        piece: Option<Piece>,
    },
}

/// Which capture a [`Step::Read`] contributes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    /// The capture's index, as [`jtag::Plan::read_dr`] returned it.
    pub capture: usize,
    /// How many bits of this read belong to it.
    pub bits: usize,
}

/// A [`jtag::Plan`] turned into control transfers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    steps: Vec<Step>,
    captures: Vec<usize>,
    flip: bool,
}

impl Program {
    /// The transfers, in order.
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// How many captures the program takes.
    #[must_use]
    pub fn capture_count(&self) -> usize {
        self.captures.len()
    }

    /// The width in bits of a capture.
    #[must_use]
    pub fn capture_bits(&self, index: usize) -> Option<usize> {
        self.captures.get(index).copied()
    }

    /// Whether the reads need [`flip_whole_bytes`] undoing on them.
    #[must_use]
    pub fn flips_whole_bytes(&self) -> bool {
        self.flip
    }

    /// Assembles the captures from the bytes each [`Step::Read`]
    /// returned.
    ///
    /// `replies` has one entry per `Read` step, in the order the steps
    /// appear; a read that is not part of a capture contributes an entry
    /// too, which is simply ignored. Each capture comes back packed
    /// least-significant bit first with the bits past its width cleared,
    /// which is the same shape [`jtag::Job::capture`] produces.
    ///
    /// # Errors
    ///
    /// [`jtag::JobError::ShortReply`] when a read returned fewer bytes
    /// than its piece needs.
    pub fn assemble(&self, replies: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, jtag::JobError> {
        let mut out: Vec<Vec<u8>> = self.captures.iter().map(|_| Vec::new()).collect();
        let mut next = 0;
        for step in &self.steps {
            let Step::Read { piece, .. } = step else {
                continue;
            };
            let reply = replies.get(next).map(Vec::as_slice).unwrap_or(&[]);
            next += 1;
            let Some(piece) = piece else { continue };
            let wanted = piece.bits.div_ceil(8);
            if reply.len() < wanted {
                return Err(jtag::JobError::ShortReply {
                    wanted,
                    got: reply.len(),
                });
            }
            let bytes = if self.flip {
                flip_whole_bytes(&reply[..wanted], piece.bits)
            } else {
                reply[..wanted].to_vec()
            };
            out[piece.capture].extend_from_slice(&bytes);
        }
        for (capture, bits) in out.iter_mut().zip(&self.captures) {
            capture.truncate(bits.div_ceil(8));
            let spare = bits.div_ceil(8) * 8 - bits;
            if spare != 0
                && let Some(last) = capture.last_mut()
            {
                *last &= 0xFF >> spare;
            }
        }
        Ok(out)
    }

    /// A capture read as a 32-bit register whose **bit 0 came out of
    /// TDO first**, which is how IEEE 1149.1 defines `IDCODE`.
    ///
    /// # Errors
    ///
    /// [`jtag::JobError::NoSuchCapture`] for an index this program does
    /// not have, [`jtag::JobError::WrongWidth`] when the capture is not
    /// 32 bits, and [`jtag::JobError::ShortReply`] as [`assemble`].
    ///
    /// [`assemble`]: Program::assemble
    pub fn capture_u32(&self, index: usize, replies: &[Vec<u8>]) -> Result<u32, jtag::JobError> {
        let bits = self
            .capture_bits(index)
            .ok_or(jtag::JobError::NoSuchCapture(index))?;
        if bits != 32 {
            return Err(jtag::JobError::WrongWidth {
                wanted: 32,
                got: bits,
            });
        }
        let captures = self.assemble(replies)?;
        let b = &captures[index];
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// Compiles a [`jtag::Plan`] into Apollo control transfers.
///
/// The plan names states; Apollo takes state numbers, so a walk is one
/// request and needs no TMS at all. A scan is the four-request dance the
/// protocol document describes — clear or fill the out buffer, scan,
/// read the in buffer — repeated once per chunk when the register is
/// longer than [`Capability::chunk_bits`], with only the last chunk
/// raising TMS on its final bit.
///
/// This performs no I/O and allocates nothing but the requests, so the
/// whole encoding is testable with no board attached, which is the same
/// discipline [`super::ftdi`] follows.
#[must_use]
pub fn compile(plan: &jtag::Plan, capability: Capability) -> Program {
    let idle = state_number(TapState::RunTestIdle);
    let mut steps = Vec::new();
    let mut captures = Vec::new();
    let goto = |steps: &mut Vec<Step>, state: TapState| {
        steps.push(Step::Command {
            request: REQUEST_JTAG_GO_TO_STATE,
            value: state_number(state),
            index: 0,
        });
    };

    for op in plan.ops() {
        match op {
            Op::Reset => {
                // Walking to Test-Logic-Reset *is* the reset, from any
                // state including an unknown one, and it is what selects
                // IDCODE on a part that has one.
                goto(&mut steps, TapState::TestLogicReset);
                goto(&mut steps, TapState::RunTestIdle);
            }
            Op::Goto(state) => goto(&mut steps, *state),
            Op::Idle(cycles) => {
                goto(&mut steps, TapState::RunTestIdle);
                let mut left = *cycles;
                while left != 0 {
                    let now = left.min(usize::from(u16::MAX));
                    steps.push(Step::Command {
                        request: REQUEST_JTAG_RUN_CLOCK,
                        value: u16::try_from(now).expect("clamped to u16::MAX"),
                        index: 0,
                    });
                    left -= now;
                }
            }
            Op::Shift {
                state,
                data,
                bits,
                capture,
            } => {
                let index = captures.len();
                if *capture {
                    captures.push(*bits);
                }
                goto(&mut steps, *state);
                let chunk = capability.chunk_bits();
                let mut done = 0;
                while done < *bits {
                    let now = (*bits - done).min(chunk);
                    let last = done + now == *bits;
                    // `chunk` is a whole number of bytes, so every chunk
                    // but the final one starts on a byte boundary and
                    // the slicing below is exact.
                    let from = done / 8;
                    let to = (done + now).div_ceil(8);
                    let slice = &data[from..to];
                    if slice.iter().all(|&b| b == 0) {
                        steps.push(Step::Command {
                            request: REQUEST_JTAG_CLEAR_OUT_BUFFER,
                            value: 0,
                            index: 0,
                        });
                    } else {
                        steps.push(Step::Write {
                            request: REQUEST_JTAG_SET_OUT_BUFFER,
                            value: 0,
                            index: 0,
                            data: if capability.flips_whole_bytes() {
                                flip_whole_bytes(slice, now)
                            } else {
                                slice.to_vec()
                            },
                        });
                    }
                    steps.push(Step::Command {
                        request: REQUEST_JTAG_SCAN,
                        value: u16::try_from(now).expect("a chunk fits in a u16"),
                        index: capability.scan_flags(last),
                    });
                    if *capture {
                        steps.push(Step::Read {
                            request: REQUEST_JTAG_GET_IN_BUFFER,
                            value: 0,
                            index: 0,
                            len: now.div_ceil(8),
                            piece: Some(Piece {
                                capture: index,
                                bits: now,
                            }),
                        });
                    }
                    done += now;
                }
                // The last chunk raised TMS, so the TAP is in Exit1-*;
                // a plan's shift ends resting in Run-Test/Idle.
                steps.push(Step::Command {
                    request: REQUEST_JTAG_GO_TO_STATE,
                    value: idle,
                    index: 0,
                });
            }
        }
    }

    Program {
        steps,
        captures,
        flip: capability.flips_whole_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apollo's state numbering, written out in full. The compiler uses
    /// a state's position in [`jtag::TAP_STATES`] as its number, which
    /// is only right while that table is in this order; this is the test
    /// that notices if it stops being.
    #[test]
    fn the_state_numbers_are_apollos() {
        let expected = [
            (0, TapState::TestLogicReset),
            (1, TapState::RunTestIdle),
            (2, TapState::SelectDr),
            (3, TapState::CaptureDr),
            (4, TapState::ShiftDr),
            (5, TapState::Exit1Dr),
            (6, TapState::PauseDr),
            (7, TapState::Exit2Dr),
            (8, TapState::UpdateDr),
            (9, TapState::SelectIr),
            (10, TapState::CaptureIr),
            (11, TapState::ShiftIr),
            (12, TapState::Exit1Ir),
            (13, TapState::PauseIr),
            (14, TapState::Exit2Ir),
            (15, TapState::UpdateIr),
        ];
        for (number, state) in expected {
            assert_eq!(state_number(state), number, "{state:?}");
            assert_eq!(
                state_from_number(u8::try_from(number).unwrap()),
                Some(state)
            );
        }
        assert_eq!(state_from_number(16), None);
        assert_eq!(state_from_number(255), None);
    }

    /// The capability reply is two little-endian words, and a short one
    /// is refused rather than half-read.
    #[test]
    fn the_capability_reply_decodes() {
        let reply = [0x00, 0x08, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00];
        let cap = Capability::from_reply(&reply).expect("eight bytes");
        assert_eq!(cap.max_scan_bits, 2048);
        assert_eq!(cap.quirks, 3);
        assert!(cap.flips_whole_bytes());
        assert!(cap.always_bitbangs());
        assert_eq!(Capability::from_reply(&reply[..7]), None);

        let plain = Capability::default();
        assert_eq!(plain.max_scan_bits, DEFAULT_MAX_SCAN_BITS);
        assert!(!plain.flips_whole_bytes());
        assert!(!plain.always_bitbangs());
    }

    /// A chunk is a whole number of bytes, never wider than `wValue`,
    /// and never zero however silly the firmware's answer.
    #[test]
    fn a_chunk_is_whole_bytes_and_fits_a_u16() {
        assert_eq!(Capability::default().chunk_bits(), 2048);
        for max in [0, 1, 7, 8, 9, 2047, 2048, 65_535, 65_536, u32::MAX] {
            let cap = Capability {
                max_scan_bits: max,
                quirks: 0,
            };
            let bits = cap.chunk_bits();
            assert!(bits >= 8, "{max} gave {bits}");
            assert!(bits.is_multiple_of(8), "{max} gave {bits}");
            assert!(u16::try_from(bits).is_ok(), "{max} gave {bits}");
            if max >= 8 {
                assert!(bits <= max as usize, "{max} gave {bits}");
            }
        }
    }

    /// The flip touches whole bytes only, and is its own inverse.
    #[test]
    fn the_flip_leaves_a_partial_byte_alone() {
        let data = [0b1000_0001, 0b0000_0011, 0b0000_0101];
        // 20 bits: two whole bytes flipped, the third left as it is.
        let flipped = flip_whole_bytes(&data, 20);
        assert_eq!(flipped, [0b1000_0001, 0b1100_0000, 0b0000_0101]);
        assert_eq!(flip_whole_bytes(&flipped, 20), data);
        // No whole bytes at all.
        assert_eq!(flip_whole_bytes(&data, 5), data);
    }

    /// The scan flags carry advance-state, and the always-bitbang quirk
    /// rides along on every scan.
    #[test]
    fn the_scan_flags_say_what_the_document_says() {
        let plain = Capability::default();
        assert_eq!(plain.scan_flags(false), 0);
        assert_eq!(plain.scan_flags(true), FLAG_ADVANCE_STATE);
        let slow = Capability {
            max_scan_bits: 2048,
            quirks: QUIRK_ALWAYS_BITBANG,
        };
        assert_eq!(slow.scan_flags(false), FLAG_FORCE_BITBANG);
        assert_eq!(
            slow.scan_flags(true),
            FLAG_ADVANCE_STATE | FLAG_FORCE_BITBANG
        );
    }

    /// The `IDCODE` plan compiles to exactly the eight transfers
    /// `docs/apollo-protocol.md` §5 lists between taking and releasing
    /// the pins, in that order. This is the test that pins the sequence
    /// the board was asked to perform.
    #[test]
    fn the_idcode_plan_compiles_to_the_documented_sequence() {
        let program = compile(&jtag::idcode_plan(), Capability::default());
        let expected = vec![
            // Reset: to Test-Logic-Reset, then rest in Run-Test/Idle.
            Step::Command {
                request: REQUEST_JTAG_GO_TO_STATE,
                value: 0,
                index: 0,
            },
            Step::Command {
                request: REQUEST_JTAG_GO_TO_STATE,
                value: 1,
                index: 0,
            },
            // The scan: into Shift-DR through Capture-DR.
            Step::Command {
                request: REQUEST_JTAG_GO_TO_STATE,
                value: 4,
                index: 0,
            },
            // Thirty-two zero bits go out, so the buffer is cleared
            // rather than filled.
            Step::Command {
                request: REQUEST_JTAG_CLEAR_OUT_BUFFER,
                value: 0,
                index: 0,
            },
            Step::Command {
                request: REQUEST_JTAG_SCAN,
                value: 32,
                index: FLAG_ADVANCE_STATE,
            },
            Step::Read {
                request: REQUEST_JTAG_GET_IN_BUFFER,
                value: 0,
                index: 0,
                len: 4,
                piece: Some(Piece {
                    capture: 0,
                    bits: 32,
                }),
            },
            Step::Command {
                request: REQUEST_JTAG_GO_TO_STATE,
                value: 1,
                index: 0,
            },
        ];
        assert_eq!(program.steps(), expected.as_slice());
        assert_eq!(program.capture_count(), 1);
        assert_eq!(program.capture_bits(0), Some(32));
    }

    /// An `IDCODE` comes back least-significant bit first, which is a
    /// little-endian `u32`. Feeding the program a synthetic reply is the
    /// whole hardware-free test of the read path.
    #[test]
    fn a_capture_decodes_least_significant_bit_first() {
        let program = compile(&jtag::idcode_plan(), Capability::default());
        let value: u32 = 0x4111_1043;
        let replies = vec![value.to_le_bytes().to_vec()];
        assert_eq!(program.capture_u32(0, &replies), Ok(value));
        assert_eq!(program.assemble(&replies).unwrap()[0], value.to_le_bytes());

        // A short reply is reported, not padded: a stalled read must not
        // look like an IDCODE of zero.
        assert_eq!(
            program.capture_u32(0, &[vec![0, 0, 0]]),
            Err(jtag::JobError::ShortReply { wanted: 4, got: 3 })
        );
        assert_eq!(
            program.capture_u32(1, &replies),
            Err(jtag::JobError::NoSuchCapture(1))
        );
    }

    /// With the flip quirk on, what the wire carries is bit-reversed and
    /// the value that comes back out is not.
    #[test]
    fn the_flip_quirk_cancels_itself_out() {
        let cap = Capability {
            max_scan_bits: 2048,
            quirks: QUIRK_FLIP_BITS_IN_WHOLE_BYTES,
        };
        let program = compile(&jtag::idcode_plan(), cap);
        assert!(program.flips_whole_bytes());
        let value: u32 = 0x4111_1043;
        let wire: Vec<u8> = value
            .to_le_bytes()
            .iter()
            .map(|b| b.reverse_bits())
            .collect();
        assert_eq!(program.capture_u32(0, &[wire]), Ok(value));

        // And a non-zero out buffer is flipped on the way out.
        let mut plan = jtag::Plan::new();
        plan.shift_dr(&[0b0000_0001, 0b0000_0010], 16);
        let program = compile(&plan, cap);
        let written = program
            .steps()
            .iter()
            .find_map(|s| match s {
                Step::Write { data, .. } => Some(data.clone()),
                _ => None,
            })
            .expect("a non-zero buffer is written");
        assert_eq!(written, vec![0b1000_0000, 0b0100_0000]);
    }

    /// A register longer than one scan is split, every chunk but the
    /// last leaves the TAP in the shift state, and the pieces reassemble
    /// into one capture.
    #[test]
    fn a_long_register_is_split_and_rejoined() {
        let cap = Capability {
            max_scan_bits: 16,
            quirks: 0,
        };
        let mut plan = jtag::Plan::new();
        let index = plan.read_dr(40);
        let program = compile(&plan, cap);
        assert_eq!(index, 0);
        assert_eq!(program.capture_bits(0), Some(40));

        let scans: Vec<&Step> = program
            .steps()
            .iter()
            .filter(|s| matches!(s, Step::Command { request, .. } if *request == REQUEST_JTAG_SCAN))
            .collect();
        assert_eq!(scans.len(), 3, "40 bits at 16 per scan");
        for (i, step) in scans.iter().enumerate() {
            let Step::Command { value, index, .. } = step else {
                unreachable!()
            };
            let last = i == 2;
            assert_eq!(*value, if last { 8 } else { 16 });
            assert_eq!(*index, if last { FLAG_ADVANCE_STATE } else { 0 });
        }

        let replies = vec![vec![0x11, 0x22], vec![0x33, 0x44], vec![0x55]];
        let joined = program.assemble(&replies).expect("three pieces");
        assert_eq!(joined[0], vec![0x11, 0x22, 0x33, 0x44, 0x55]);
    }

    /// Bits past a capture's width are cleared rather than left as
    /// whatever the firmware's buffer held.
    #[test]
    fn a_partial_last_byte_is_masked() {
        let mut plan = jtag::Plan::new();
        let _ = plan.read_dr(6);
        let program = compile(&plan, Capability::default());
        let joined = program.assemble(&[vec![0xFF]]).expect("one byte");
        assert_eq!(joined[0], vec![0b0011_1111]);
    }

    /// A walk that a plan collapses (the TAP is already there) costs no
    /// request, and an idle longer than a `wValue` becomes several.
    #[test]
    fn walks_and_idles_compile_to_the_fewest_requests() {
        let mut plan = jtag::Plan::new();
        plan.goto(TapState::TestLogicReset);
        let program = compile(&plan, Capability::default());
        assert!(program.steps().is_empty(), "already in Test-Logic-Reset");

        let mut plan = jtag::Plan::new();
        plan.idle(70_000);
        let program = compile(&plan, Capability::default());
        let clocks: usize = program
            .steps()
            .iter()
            .filter_map(|s| match s {
                Step::Command { request, value, .. } if *request == REQUEST_JTAG_RUN_CLOCK => {
                    Some(usize::from(*value))
                }
                _ => None,
            })
            .sum();
        assert_eq!(clocks, 70_000);
    }

    /// The request numbers are the ones `docs/apollo-protocol.md` gives,
    /// spelled out once so a typo in a constant is a failing test rather
    /// than a board that does not answer.
    #[test]
    fn the_request_numbers_are_the_documented_ones() {
        assert_eq!(REQUEST_ADVERTISEMENT_STOP, 0xF0);
        assert_eq!(REQUEST_GET_ID, 0xA0);
        assert_eq!(REQUEST_GET_FIRMWARE_VERSION, 0xA2);
        assert_eq!(REQUEST_GET_USB_API_VERSION, 0xA3);
        assert_eq!(REQUEST_ALLOW_FPGA_TAKEOVER_USB, 0xC2);
        assert_eq!(REQUEST_JTAG_CLEAR_OUT_BUFFER, 0xB0);
        assert_eq!(REQUEST_JTAG_SET_OUT_BUFFER, 0xB1);
        assert_eq!(REQUEST_JTAG_GET_IN_BUFFER, 0xB2);
        assert_eq!(REQUEST_JTAG_SCAN, 0xB3);
        assert_eq!(REQUEST_JTAG_RUN_CLOCK, 0xB4);
        assert_eq!(REQUEST_JTAG_GO_TO_STATE, 0xB5);
        assert_eq!(REQUEST_JTAG_GET_STATE, 0xB6);
        assert_eq!(REQUEST_JTAG_GET_INFO, 0xB8);
        assert_eq!(REQUEST_JTAG_STOP, 0xBE);
        assert_eq!(REQUEST_JTAG_START, 0xBF);
        // Direction and recipient, which are the other half of a
        // request: a vendor read is device to host, and the one
        // interface-recipient request is the handover.
        assert_eq!(REQ_TYPE_OUT & 0x80, 0);
        assert_eq!(REQ_TYPE_IN & 0x80, 0x80);
        assert_eq!(REQ_TYPE_OUT & 0x1F, 0, "recipient device");
        assert_eq!(REQ_TYPE_IN & 0x1F, 0, "recipient device");
        assert_eq!(REQ_TYPE_OUT_INTERFACE & 0x1F, 1, "recipient interface");
        assert_eq!(REQ_TYPE_OUT & 0x60, 0x40, "vendor");
    }

    /// None of the requests this project refuses to send appears in a
    /// compiled program. `0xC0` reconfigures the part and `0xC1` holds
    /// it offline; neither belongs in a read.
    #[test]
    fn nothing_that_acts_on_the_fpga_is_ever_compiled() {
        let mut plan = jtag::Plan::new();
        plan.reset();
        plan.shift_ir(&[0x06], 8);
        let _ = plan.read_dr(32);
        plan.idle(100);
        let program = compile(&plan, Capability::default());
        for step in program.steps() {
            let request = match step {
                Step::Command { request, .. }
                | Step::Write { request, .. }
                | Step::Read { request, .. } => *request,
            };
            assert!(
                !matches!(request, 0xC0 | 0xC1 | 0xA1),
                "a program must never carry {request:#04x}"
            );
        }
    }
}
