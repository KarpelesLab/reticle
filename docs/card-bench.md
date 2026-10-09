# Driving a smart card and its display from a Basys 3

What another machine needs to repeat this. The pin map and the safety rules
are in `examples/basys3/iso_display.rcf`, which is authoritative and has the
provenance; this file is the operating knowledge around it, most of which was
learned by getting it wrong.

## Safety, first

**`N2` (JXADC4) drives an AQV210 PhotoMOS relay that switches the device's
3.3 V supply.** High closes the relay and powers somebody's hardware.

- It must be low at power-up, and only the card half's activation sequence
  may raise it. Nothing else in a design may reach that ball.
- **Always send `D` when finished.** The terminal also deactivates by itself
  after about nineteen seconds of silence, which is a backstop and not a
  policy.
- **Do not power-cycle the device repeatedly without being asked.** A loop
  that activates and deactivates switches the relay each time; eight
  iterations alarmed the board's owner, reasonably.
- One agent at a time on the JTAG chain.

## The wiring

| header | ball | signal |
|---|---|---|
| JXADC1 | `J3` | the card's single contact, open drain, **1 kΩ pull-up to 3.3 V** on the board |
| JXADC2 | `L3` | card clock, 8.000 MHz (CHECKED: 125.0 ns, 50 % duty) |
| JXADC3 | `M2` | card reset, active low |
| JXADC4 | `N2` | `vcc_en`, the relay |
| JA1–JA4 | `J1 L2 J2 G2` | `sclk`, `mosi`, `dc`, `cs_n` from the device's display bus |
| JC1, JC2 | `K17`, `M18` | the device's LEFT and RIGHT button lines |

A 4.7 kΩ pull-up is **not** adequate: against 50 pF it reaches 90 % in about
540 ns, and an etu at 2 Mbaud is 500 ns. 1 kΩ gives about 115 ns.

## Building it

```sh
RETICLE_CHIPDB=/path/to/prjxray-db reticle fpga \
    --device xc7a35t-cpg236 \
    --constraints examples/basys3/iso_display.rcf \
    --bitstream iso_display.bit \
    examples/basys3/iso_display_pad.v examples/basys3/iso_display.v \
    examples/basys3/iso7816_terminal.v examples/basys3/ssd1306_console.v \
    ip/bus/uart/rtl/uart.v ip/bus/uart/rtl/uart_tx.v \
    ip/bus/uart/rtl/uart_rx.v ip/bus/uart/rtl/uart_baud_div.v \
    ip/bus/iso7816_uart/rtl/iso7816_uart.v \
    ip/bus/seph_mcu/rtl/seph_mcu.v \
    ip/bus/spi_display_rx/rtl/spi_display_rx.v \
    ip/video/ssd1306_slave/rtl/ssd1306_slave.v \
    ip/util/cdc_sync/rtl/cdc_sync.v
```

`BLOCK_RAM` defaults to 1 and the frame buffer is a real block RAM. It
briefly had to be 0: a `din` bit on a port the design only reads through
could not find a free `GND_WIRE` fan, and whether it could depended on
placement. That is fixed -- a data bit nothing stores or reads no longer
fails a build.

## Two rates, and why they are what they are

**The card runs at 1 Mbaud, not 2.** Its ATR offers `TA1 = 87`, which is
F/D = 4 and 2 Mbaud on the contact -- but `87` is vendor-specific (FI = 8 is
RFU) and at that rate a card byte lands every 6 us while printing it costs
10 us on the console, so a long reply overflowed the receive buffer and the
card's answer to its initialisation frame lost 7 bytes. `97` is the
standard's own pair -- FI = 9 is Fi = 512, DI = 7 is Di = 64, F/D = 8 -- and
this device accepts it. A byte then lands every 12 us against 10 us to print
it, and nothing is lost. `PPS1` and `FAST_ETU_CYCLES` are parameters and
**must agree**.

Measured by `examples/basys3/iso7816_burst_tb.v`, which prints and never
asserts:

    F/D = 4 (PPS1 87, 2 Mbaud)   24 bytes: 10 lost   200 bytes: 85 lost
    F/D = 8 (PPS1 97, 1 Mbaud)   24 bytes:  0 lost   200 bytes:  0 lost

## The console is 2.000 Mbaud

Not 115200. `HOST_DIV = 56` against a 112 MHz PLL, zero error; Linux has
`B2000000`. At 115200 a received card byte cost 260 us to print against 6 us
between arrivals at F/D = 4, and the card's waiting time at that rate is
about 4.8 ms -- a host could not be told in time, let alone answer.

Verify the link before trusting anything through it: `k` emits
`0123456789ABCDEFFEDCBA9876543210` and it must come back exactly.

## The sequence

```
A                 activate: VCC, clock, reset released after 400 card clocks
                  -> +VCC +CLK +RST, then the card's ATR as hex
P                 send PPS FF 10 97 78 (`PPS1`); the echo comes back as hex
F                 switch to the fast rate -- only after checking the echo
:<hex>            send those bytes to the card
W                 become the SE's MCU: `ip/bus/seph_mcu` sends SESSION_START and
                  answers every SE turn from then on (tickers, status, BLE command
                  completes, buttons from `<` and `>`); only after `F`
D                 deactivate: reset, clock, line, then power, in that order
s                 card status, 32 hex characters
?  g              display status; dump the frame buffer as hex
m  u              mute / unmute the printing of card bytes
```

### The initialisation frame

51 bytes, which this device answers by clearing its display and then drawing
a logo. Send them with `:` and a carriage return; the parser takes `0-9 A-F
a-f` and the case does not change what reaches the contact.

```
010030000800030304322e323804f4d8aa4304312e313604f1308974100702020014150c090106312e31312e3005312e322e30
```

```
01 00 30 00 08 00 03 03 04 32 2e 32 38 04 f4 d8 aa 43 04 31 2e 31 36 04 f1
30 89 74 10 07 02 02 00 14 15 0c 09 01 06 31 2e 31 31 2e 30 05 31 2e 32 2e 30
```

Several of those bytes are ASCII -- `32 2e 32 38` is `2.28`, `31 2e 31 36`
is `1.16`, `31 2e 31 31 2e 30` is `1.11.0` and `31 2e 32 2e 30` is `1.2.0` --
so it looks like a set of version strings the device is told about. A wrong
byte does not trigger initialisation at all, which is what makes the display
drawing a sufficient check that all 51 arrived.

**`F` must be conditional on the echo matching what was sent** (`FF109778`
with the default `PPS1`). Switching
regardless leaves this end at 2 Mbaud against a card still at 21505 baud,
mutually deaf, and it looks like the card ignoring what follows rather than a
negotiation that never happened. One run drew a logo and the next did not,
for exactly that reason.

## Gotchas that cost hours

- **Drain the port to quiet before measuring.** A frame dump is 2064
  characters and the terminal may still be sending it when the next command
  goes out, so the previous run's pixels arrive where the ATR was expected.
- **Hex characters inside a `:` run used to run as commands.** Fixed, but
  worth knowing why a frame containing `d8`, `f4` or `0c` once deactivated
  the card mid-frame: uppercase `D` is deactivate. The bytes are the same
  either case; the parser takes `0-9 A-F a-f`.
- **`lost_q` in `s` must read 0.** It counts received bytes the buffer had
  to drop. At F/D = 4 a long reply lost 7 of them; at F/D = 8 it loses none,
  which is why the terminal asks for `97`. If it is not zero, the console is
  not keeping up with the contact and the two rates have drifted apart.
  The reply carries `60`, T=0's NULL procedure byte asking for more time, so
  answering it needs all of it. Three receive queues were written for this
  before the rate was changed instead, and all three were reverted; see
  `iso7816_burst_tb.v`, which reproduces the loss, and `partsel.v`, which
  clears the construct that was suspected.
- **A loose SPI wire reads as `unknown` climbing while `data` climbs too.**
  An undriven `mosi` floats high, so every byte is `FF`: not a valid command,
  and the frame buffer fills with ones. Check `?` before trusting a display
  result.
- **The bench scripts are not in the repository.** `tools/local/` is
  deliberately ignored. The sequence above is all a replacement needs.

## Open: some builds misbehave on the part, and which ones moves with placement

**NOT EXPLAINED, 10 October 2026.** The same RTL, built at different
placements, has given on this board: `TTTT` instead of `+VCC`; a PPS sent
as `FF FF FF ...`; banners and line endings repeated hundreds of times; and
no answer at all. All of these are logic the testbenches pass. One build
was taken apart: bit 3 of the card half's output byte (`host_data[3]`) is
stuck at zero -- `k` prints `8` as `0` and CR LF as `05 02`, while the
display half on the same port prints cleanly. On that build every
inter-tile join (109 921) matches Project X-Ray's `tileconn.json`, every
fixed intra-tile hop (28 203) its `ppips`, the flip-flop's slice features
are self-consistent, no lookup table depends on a tied input, and **the
fault is unchanged at half the clock with identical placement and
routing**, so it is not timing. What is left is something the database
cannot check from inside the flow -- the frame address a tile's bits land
at, or the database itself. Until it is found, a build has to be checked
with `k`, `?`, `A`, `P` before it is trusted with the device.

## What is CHECKED on hardware

Activation in the mandated order; a real card's 14-byte ATR
(`3B1B8705322E352E310433000004`) with no parity or framing errors; a PPS the
card echoes exactly; the rate change; a 51-byte initialisation frame the card
acts on; and the device drawing into `ip/video/ssd1306_slave` -- 3072 bytes
through 83 commands, **zero unknown commands**, four consecutive identical
runs. The card clock measured 8.0000 MHz with a logic analyser.

Not established on hardware at the time of writing: the 1 Mbaud rate. The
`97` exchange and its loss figures are from `iso7816_burst_tb.v` and the
card owner's word that the device accepts it; the hardware runs above were
taken at F/D = 4.
