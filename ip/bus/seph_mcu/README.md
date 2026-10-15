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
   `aci_gatt_add_char` (`fd04`) are reproduced. A characteristic's handle
   advances by two, or by three if its properties include notify or
   indicate (`0x30`), as the real radio's did.
2. **STATUS_EVENT** (`0x15`), the `STATUS` parameter with the USB and BLE
   flags (USB, below) added to its low flag byte, for a REQUEST_STATUS
   (`0x52`).
3. **USB_EP_XFER_EVENT** (`0x10`) "IN done" for each IN transfer the SE
   prepares (USB_EP_PREPARE `0x50`, type `0x20`), endpoint 0 first.
4. **USB_EVENT** (`0x0F`) reset, once, when `usb_plugged` is high and the
   SE has connected (USB_CONFIG `0x4F`, `01`).
5. **An injected event**, verbatim: whatever the host behind `inj_*` wants
   the SE to see.
6. **BUTTON_PUSH_EVENT** (`0x05`) when `buttons` differs from what was
   last reported: `buttons << BUTTON_SHIFT`.
7. **TICKER_EVENT** (`0x0E`), milliseconds since `start`, once the SE's
   interval has passed since the last one. SET_TICKER_INTERVAL (`0x4E`)
   sets it; 100 ms until then.

If nothing is due the turn waits. Every other SE command — the rest of
USB configuration, MCU lock, MORE_TIME — is counted and needs no answer.

### USB

This block is not a USB host; it is the MCU between the SE and one. In the
capture everything the MCU sends for USB is one of four things, and this
block does three of them itself:

| event | when | done by |
|---|---|---|
| USB_EVENT reset `0f 00 01 01` | a host sees the device: cable in and SE connected | this block, once per plug |
| "IN done" `10 00 03 <ep, bit 7 set> 02 <len>` | after each IN the SE prepares | this block, on its next turn |
| STATUS's USB flags | always | this block: `0x09` cable in, `0x02` added after the reset |
| SETUP and OUT transfers `10 …` | whenever the host has something to say | **the host**, through `inj_*` |

An IN completion carries the prepared length on endpoint 0 and **zero** on
any other endpoint, because that is what the capture's MCU sent: 1632 of
1632 on endpoint 2 say length 0 after 64 bytes.

The data the SE prepares is not repeated anywhere: it is in the SE's own
USB_EP_PREPARE, which whoever carries this block's bytes already sees.

**The injected event is checked, not trusted.** `inj_start`, then one
`inj_valid` per byte, then `inj_commit`; the commit is refused with a
one-cycle `inj_bad`, and nothing is sent, unless the bytes are exactly one
packet — a tag, a length, and that many bytes — and nothing else is
waiting. A packet out of step with its length would put the link out of
step with the SE for good. Up to 2047 bytes; a HID OUT is 70. (The buffer is 2048 so that it fills a block RAM exactly: at 256, a block RAM address pin the design left undriven could not be tied on `iso_display.v`, because the router had used its ground fan.)

With the cable out, nothing happens on USB at all, which is what a real MCU
does with no host: the SE's USB_CONFIG `connect` goes unanswered.

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
an SE 2.5.1 accepted: MCU "2.28", bootloader "1.16", and a status of on
battery, not charging, 80 %, 4000 mV -- flags 0 -- to which the block adds
BLE on (`0x04`) once the SE has sent its first BLE command, as a real MCU
does.

The status used to say USB-powered and charging (flags `0x09`), so that the
SE would neither expect a USB host nor warn about the battery. A real MCU
never reports that: in the capture's 586 statuses USB-powered appears only
with USB on or a host's bus reset about to make it so, and BLE on is set
whenever the SE has started the radio. With `0x09`, an SE 2.5.1 took PIN
digits and then went back to "Welcome" at the onboarding's PIN
confirmation. **The status was not the cause**: with flags 0 the same SE
went back to "Welcome" at the same step, with the same traffic. The
default stays as it is because it is what a real MCU says; the restart is
still open. (A bench log that appeared to show the SE sending one more BLE
command after the change, `aci_gap_init`, did not: the earlier log had it
too, on one line with two other packets, and its parser had skipped it.)

## 3. What is CHECKED, and what is QUOTED

**CHECKED on a real SE 2.5.1, 9 October 2026**, through a host-side
prototype of exactly this policy over the Basys 3 console
(`examples/basys3/iso_display.v`): SESSION_START, then 182 tickers and 18
status events in 20 s, every one answered by a status, and the SE drew its
PIN entry screen.

**CHECKED on a part, 10 October 2026: this block itself**, as the console's
`W` command in `examples/basys3/iso_display.v` on a Basys 3. After `A`, `P`
and `F`, with no host involved, it sent SESSION_START and answered every SE
turn -- 78 tickers and 7 status events in 7 s, a status after each -- and the
SE drew its PIN screen into the board's SSD1306 slave, 4096 bytes with no
unknown command. (Earlier builds of that design misbehaved for a reason in
the router, not here: see `docs/card-bench.md`.)

**QUOTED, not measured:**

- the BLE and USB configuration — answered byte for byte as a real Nano
  X's MCU answered them in a capture (`visgrok`,
  `capture-20261009-125946.seph.log`, SE 2.6.0), which
  `rtl/seph_mcu_replay_tb.v` replays. No SE has sent either to this block:
  an SE 2.5.1 at its PIN screen sent neither BLE_SEND nor USB_CONFIG in
  60 s, where the SE 2.6.0 of the capture configured both 2 s after
  booting. Whether 2.5.1 waits for the PIN, or for something in
  SESSION_START (its features word is `08000303`, the capture's
  `08002303`), is not known;
- `BUTTON_SHIFT = 1` — Ledger's SDK reads the button byte as `byte >> 1`.
  **A Nano X does not use BUTTON_PUSH_EVENT at all**: its MCU sends none in
  the 44 143 packets of the capture, a PIN entered during it, because the
  buttons are wired to the SE. An SE 2.5.1 sent four of them from this
  block answered each with a plain status and changed nothing on screen.
  `iso_display.v` therefore ties `buttons` low and presses the button lines
  instead;
- what an SE does with a status it does not like.

## 4. The testbenches

`rtl/seph_mcu_tb.v` plays a scripted SE and checks every byte the block
sends: SESSION_START byte for byte, nothing before the SE's first status
and one event per turn after it, tickers at the SE's interval by their own
timestamps, the status packet, two BLE replies byte for byte against the
capture, a button press and release, and the counters. It is one boot, not
the protocol: an SE that wants something the script does not ask for is
not covered.

`rtl/seph_mcu_replay_tb.v` replays a real configuration from the capture:
every packet the SE sent from SESSION_START until its fifteenth BLE command
was answered, USB_CONFIG included, and each BLE answer must equal the real
MCU's byte for byte. It caught the characteristic handles, which the block
counted two at a time and so answered `0d 0f 11` where the radio had said
`0d 10 12`.

`rtl/seph_mcu_usb_tb.v` drives USB with the capture's own packets: the
reset only once the cable is in and the SE connected, and again after the
cable goes out and back; the status flags at each step; SET_ADDRESS,
GET_DESCRIPTOR and a 70-byte HID OUT injected and sent byte for byte; each
IN completed as the real MCU completed it; and a malformed injection, and
one made while another waited, refused without disturbing the one waiting.
