# seph_mcu

The MCU's end of Ledger's SEPROXYHAL link: enough of it to bring a Nano X's
secure element (SE) up and keep it running, with the SE on an ISO 7816
contact and this block in place of the STM32 that normally talks to it.

It moves **bytes**. `iso7816_uart` (or anything with a byte stream and a
valid/ready handshake) carries them; this block knows nothing about
characters, parity or the PPS that sets the rate before it starts.

## 1. The protocol, as far as this block goes

A packet is a tag, a 16-bit big-endian length and that many bytes. The link
is **turn based**:

| who | tags | what |
|---|---|---|
| MCU | `0x01`–`0x1F` | **one** event per turn |
| SE | `0x30`–`0x5F` | any number of commands |
| SE | `0x60`–`0x6F` | a status, which **ends the SE's turn** |

`start` sends SESSION_START without waiting for a turn. After that, on
each turn, the block sends the first of these that is due:

1. **BLE_RECV_EVENT** (`0x18`) carrying HCI "command complete", status
   success, for a BLE_SEND (`0x38`). The SE configures a radio and waits
   for each command's completion; answering success accepts the
   configuration without a radio. The return parameters a real MCU sent
   for `aci_gap_init` (`fc8a`), `aci_gatt_add_service` (`fd02`) and
   `aci_gatt_add_char` (`fd04`) are reproduced.
2. **STATUS_EVENT** (`0x15`), the `STATUS` parameter verbatim, for a
   REQUEST_STATUS (`0x52`).
3. **BUTTON_PUSH_EVENT** (`0x05`) when `buttons` differs from what was
   last reported: `buttons << BUTTON_SHIFT`.
4. **TICKER_EVENT** (`0x0E`), milliseconds since `start`, once the SE's
   interval has passed since the last one. SET_TICKER_INTERVAL (`0x4E`)
   sets it; 100 ms until then.

If nothing is due the turn waits. Every other SE command — USB
configuration, MCU lock, MORE_TIME — is counted and needs no answer.

## 2. Parameters

| parameter | default | meaning |
|---|---|---|
| `MS_CYCLES` | 112000 | system clocks per millisecond |
| `SESSION`, `SESSION_LEN` | 51 bytes | the whole SESSION_START packet, right-aligned in 512 bits |
| `STATUS`, `STATUS_LEN` | 23 bytes | the whole STATUS_EVENT packet, likewise |
| `BUTTON_SHIFT` | 1 | where the button bits go in the event's byte |

**`SESSION` and `STATUS` depend on the SE.** SESSION_START names the MCU's
firmware and bootloader versions, and an SE checks them: a capture shows
an SE 2.8.0 ordering an MCU at 2.39.0 into its bootloader to be updated.
STATUS_EVENT's layout differs between MCU versions. The defaults are what
an SE 2.5.1 accepted: MCU "2.28", bootloader "1.16", and a status of
USB-powered, charging, 80 %, 4000 mV, chosen so the SE neither expects a
USB host nor warns about the battery.

## 3. What is CHECKED, and what is QUOTED

**CHECKED on a real SE 2.5.1, 9 October 2026**, through a host-side
prototype of exactly this policy over the Basys 3 console
(`examples/basys3/iso_display.v`): SESSION_START, then 182 tickers and 18
status events in 20 s, every one answered by a status, and the SE drew its
PIN entry screen.

**NOT yet run on a part: this block itself.** It is integrated into
`examples/basys3/iso_display.v` as the console's `W` command and passes that
design's testbenches, but the builds made of it so far have hit a
placement-dependent fault elsewhere in the card half (see
`docs/card-bench.md`), so the engine has not yet talked to the SE. What
was checked on the part is the same policy driven from the host.

**QUOTED, not measured:**

- the BLE answers — from a capture of a real Nano X's MCU
  (`visgrok`, `capture-20261009-125946.seph.log`); the SE 2.5.1 above
  sent no BLE command;
- `BUTTON_SHIFT = 1` — Ledger's SDK reads the button byte as `byte >> 1`;
  no button press has reached an SE from this block yet;
- what an SE does with a status it does not like.

## 4. The testbench

`rtl/seph_mcu_tb.v` plays a scripted SE and checks every byte the block
sends: SESSION_START byte for byte, nothing before the SE's first status
and one event per turn after it, tickers at the SE's interval by their own
timestamps, the status packet, two BLE replies byte for byte against the
capture, a button press and release, and the counters. It is one boot, not
the protocol: an SE that wants something the script does not ask for is
not covered.
