//! An 8N1 receiver that reads bytes off a *waveform*, shared by the
//! tests that check what a system says on its serial pin.
//!
//! Both worked examples — `examples/soc` and `examples/mos6502_computer`
//! — end at the same place: a wire, a baud rate, and a string that must
//! come out of it. The decoder here is the one that proves it, and it is
//! deliberately written from the frame rather than from the library's
//! transmitter: it takes the list of changes of one net, finds each
//! falling edge from idle, samples the middle of every bit at the
//! nominal bit time, and insists on the start and stop bits. A
//! transmitter that shifted its bits in the wrong order, held a bit for
//! the wrong number of clocks or dropped the stop bit would be caught by
//! it, which is the point of not reusing `ip/uart`'s receiver here.
//!
//! The testbenches do the same job in Verilog and print what they
//! receive; each test checks that the two agree.

#![allow(dead_code)]

/// A net's changes, as `(time, level)` in the order they happened, with
/// `None` for a level that is not 0 or 1.
pub(crate) type Waveform = Vec<(u64, Option<bool>)>;

/// Decodes 8N1 frames from the transitions of a serial line.
///
/// A frame starts at a falling edge from idle; each bit is sampled in its
/// middle, `bit` ticks apart, and the start bit must still be low there
/// and the stop bit high.
pub(crate) fn decode_uart(edges: &[(u64, Option<bool>)], bit: u64) -> Result<Vec<u8>, String> {
    let level_at = |t: u64| -> Option<bool> {
        edges
            .iter()
            .take_while(|(when, _)| *when <= t)
            .last()
            .and_then(|(_, level)| *level)
    };
    let mut out = Vec::new();
    let mut from = 0u64;
    loop {
        let start = edges
            .windows(2)
            .find(|w| w[1].0 >= from && w[0].1 == Some(true) && w[1].1 == Some(false))
            .map(|w| w[1].0);
        let Some(start) = start else { break };
        let sample = |k: u64| level_at(start + k * bit + bit / 2);
        if sample(0) != Some(false) {
            return Err(format!("a start bit at {start} is not low in its middle"));
        }
        let mut byte = 0u8;
        for k in 0..8 {
            match sample(k + 1) {
                Some(true) => byte |= 1 << k,
                Some(false) => {}
                None => return Err(format!("data bit {k} of the frame at {start} is x")),
            }
        }
        if sample(9) != Some(true) {
            return Err(format!("the frame at {start} has no stop bit"));
        }
        out.push(byte);
        from = start + 9 * bit + bit / 2;
    }
    Ok(out)
}
