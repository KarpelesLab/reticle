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
| JC2 | `M18` | the device's LEFT button line: idles low, a press drives it high (CHECKED) |
| JC1 | `K17` | the device's RIGHT button line: idles high, a press drives it low (CHECKED) |

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
                  completes); only after `F`. A Nano X's buttons are its own
                  lines, not SEPROXYHAL events -- see below
<  >              press the LEFT / RIGHT button for 100 ms
[  ]              push the LEFT button and hold it / release it
{  }              the same for the RIGHT button; `D` releases both
D                 deactivate: reset, clock, line, then power, in that order
s                 card status, 32 hex characters
?  g              display status; dump the frame buffer as hex
m  u              mute / unmute the printing of card bytes
U  X              a USB cable in / out (+USB / -USB): with it in, the engine
                  resets the bus once the SE connects, completes every IN the
                  SE prepares, and the status says USB powered and on
!<hex>            one SEPROXYHAL event for the engine's next turn, ended like
                  a `:` run: a host's SETUP or OUT transfer, for example
                  `!10000B0001088006000100001200` (GET_DESCRIPTOR, device,
                  18 bytes); -INJ if refused. What the SE answers comes back
                  as its USB_EP_PREPARE in the hex like everything it sends
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

## USB over the console

`U`, `X` and `!` let a host at the console be the device's USB host. Nothing
here is a USB controller: the SE has its own USB stack, a real Nano X's MCU
only carries packets between it and the wire, and so does this.

**Status: CHECKED in simulation against the capture's own packets; not yet
run against an SE that turned USB on.** On 16 October 2026 an SE 2.5.1 at
boot was given `U` and sent no USB_CONFIG at all in 21 s, so the engine
correctly sent nothing. That SE did configure USB after the onboarding
restarts of 15 October. The first run against a connecting SE is the test
this section is written for: please record what happens here, including
where it differs from what follows.

### Who does what

Every USB packet in the capture (`visgrok`,
`capture-20261009-125946.seph.log`) is one of these. The engine
(`ip/bus/seph_mcu`) does the first three by itself; the host does the
fourth with `!`.

| packet | bytes | when | done by |
|---|---|---|---|
| USB_EVENT reset | `0F 00 01 01` | cable in (`U`) **and** the SE has sent USB_CONFIG connect `4F 00 01 01`; once per plug | engine |
| "IN done" | `10 00 03 80 02 <len>` on endpoint 0, `10 00 03 82 02 00` on endpoint 2 | the turn after each IN the SE prepares | engine |
| status flags | STATUS_EVENT byte 6: `09` cable in, `0B` after the reset, `0F` with BLE on too | every REQUEST_STATUS | engine |
| SETUP, OUT | `10 00 0B 00 01 08 <8-byte setup>`; `10 00 43 82 04 40 <64 bytes>` | whenever the host has something to say | **host**, with `!` |

The SE's answers are its own packets and come back in the card hex like
everything else it sends (keep `u`, not `m`):

| SE packet | bytes | meaning |
|---|---|---|
| USB_CONFIG | `4F 00 01 01` connect, `4F 00 01 02` disconnect, `4F 00 02 03 <addr>` address, `4F 00 05 04 …` endpoints | needs no answer |
| USB_EP_PREPARE, IN | `50 <len16> <ep> 20 <n> <n bytes>` | data for the host: **this is the answer**. `<ep>` is `00` for endpoint 0, `82` for HID endpoint 2 |
| USB_EP_PREPARE, ready | `50 00 03 <ep> 30 <n>` | the SE will take an OUT of up to `n` bytes; needs no answer |

### Sending: `!`

`!`, then the event as hex, then CR. Like a `:` run it holds the display
half off and ends on any non-hex character. The engine sends it whole on
its next turn, ahead of the ticker; the host sees nothing for that, since
the monitor does not print the engine's bytes while `W` runs.

`-INJ` and nothing sent when the event is not exactly one packet (the
length field must equal the bytes after it), when the digits are odd,
when `W` is not running, or when an earlier `!` event has not gone out
yet. Wait for the SE's answer before the next one; the engine holds one.

### Enumerating, step by step

After `A`, `P` (echo `FF109778`), `F`, `W`, then `U` (`+USB`). Wait for
`4F 00 01 01` from the SE; the engine resets the bus by itself, and the SE
then sends `4F 00 05 04 01 00 01 40` and `4F 00 05 04 01 80 01 40`
(endpoint 0, both directions, 64 bytes). Then, as the capture's host did,
each line one `!` event, waiting for the IN after each:

| send | the SE should prepare |
|---|---|
| `!10000B0001080005010000000000` SET_ADDRESS 1 | `4F 00 02 03 01`, then `50 00 03 00 20 00` (status stage) |
| `!10000B0001088006000100000800` device descriptor, 8 | `50 00 0B 00 20 08 12 01 10 02 00 00 00 40` |
| `!10000B0001088006000100001200` device descriptor, 18 | 18 bytes: VID `2c97`, PID `4000` on the capture's device |
| `!10000B0001088006020309040E00` string 2 | `"Nano X"` in UTF-16 |
| `!10000B0001088006010309040E00` string 1 | `"Ledger"` |
| `!10000B0001088006000200000900` configuration, 9 | its total length in bytes 2-3 |
| `!10000B0001088006000200006000` configuration, 96 | **two** INs, 64 then 32: see below |
| `!10000B0001080009010000000000` SET_CONFIGURATION 1 | `50 00 03 00 20 00` |

Each SETUP is `10 00 0B 00 01 08` then the eight setup bytes as USB sends
them: bmRequestType, bRequest, wValue, wIndex, wLength, little-endian.

**An IN longer than 64 bytes comes in pieces.** The SE prepares 64, the
engine completes it, and the SE prepares the rest on its next turn. Keep
reading INs on that endpoint until one is shorter than 64 or the total
reaches wLength.

### An APDU over HID

Ledger's HID framing on endpoint 2, 64-byte reports: channel (2 bytes),
tag `05`, sequence number (2 bytes, from 0), and in the first report only
the APDU's length (2 bytes); then the APDU, continued in later reports, the
last padded with zeros. Each report is one OUT event:

```
!10004382 04 40 <64-byte report>          (spaces here for reading only)
```

The capture's host sent GET_APP_NAME_AND_VERSION (`B0 01 00 00 00`) on
channel `09DD`:

```
!100043820440 09DD050000 0005 B001000000 <zeros to 64 bytes>
```

and the SE answered with `50 00 03 02 30 40` (ready for the next) and an
IN on `82` carrying the same framing: `09DD050000 0002 5515` -- two bytes,
status word `5515`, device locked. The engine completes it with
`10 00 03 82 02 00` by itself. Responses longer than one report arrive as
several INs on `82`, sequence numbers counting up.

### If the SE never connects

What an SE does with a bus reset it did not connect for has not been
seen. `!0F000101` sends one by hand -- the engine does not refuse it -- and
the status still says the cable is in. If the SE answers descriptor
requests after that, USB works without its own connect; if not, it needs
whatever state makes it connect (the capture's SE 2.6.0 connected 2 s after
boot; the bench's 2.5.1 has done so only after an onboarding restart).

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

## Builds that misbehaved on the part: two signals on one long line

**EXPLAINED AND FIXED, 10 October 2026.** The same RTL, built at different
placements, gave on this board: `TTTT` instead of `+VCC`; a PPS sent as
`FF FF FF ...`; banners and line endings repeated hundreds of times; and no
answer at all -- all logic the testbenches pass. One build was taken apart:
`k` printed `8` as `0` and CR LF as `05 02`, bit 3 of the card half's output
byte gone, while the display half on the same port printed cleanly.

Every check inside the flow passed: 109 921 inter-tile joins matched
Project X-Ray's `tileconn.json`, 28 203 fixed hops its `ppips`, no lookup
table depended on a tied input, and the fault was unchanged at **half the
clock with identical placement and routing**, so it was not timing.
Configuration readback (`reticle program --readback`, written for this)
showed the part held the bitstream word for word, top and bottom halves,
and `--capture` showed the flip-flop itself holding the right value -- so
the value was lost on its way *out*.

The cause was in the router. A 7-series long line (`LV`, `LVB`, `LH`) is one
piece of metal with a different name in every tile, joined in the graph by
a free pip each way, and the router counted capacity per **name**. A long
line can be driven from either end, so it put one signal on `LV0` and
another on `LV18` of the same wire: 38 such pairs in that build, each route
valid on its own, shorted on the part. `route::physical_nodes` now merges
joined names into one node for capacity, and `reticle fpga --bitstream`
refuses a routing in which any physical wire carries two signals. The same
placement then printed `k` exactly and passed every burst.

## What is CHECKED on hardware

**The FPGA as the SE's MCU, 10 October 2026.** `A`, `P`, `F`, then `W`: the
board alone ran the secure element -- SESSION_START, then a ticker or a
status event for every turn the SE ended -- and the SE drew its PIN screen,
read back with `g`. `<` and `>` move the PIN cursor left and right: each
press is 100 ms on the button line, JC2 driven high for left and JC1 driven
low for right, which is the opposite pairing to what the constraints said
until both lines were read back idle and a press was watched on the screen.

**Held presses, 11 October 2026.** `[` `]` `{` `}` hold and release each
button for as long as the host likes, both at once included, with the lines'
levels read back through `s` at every step. Held 1.5 s, neither button moved
the cursor until it was released -- the device acts on the release -- and
`[{` then `]}` entered a digit: the first PIN dot filled and the delete glyph
appeared. `D` releases anything still held, because it powers the device
down and a held line would drive an unpowered input.


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
