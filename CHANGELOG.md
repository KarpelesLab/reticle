# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.6](https://github.com/KarpelesLab/reticle/compare/v0.0.5...v0.0.6) - 2026-10-09

### Added

- *(basys3)* the PPS exchange and 2 Mbaud, on a real card
- *(fpga)* a device file may name the pins whose constant it ties itself
- *(basys3)* an ISO 7816 terminal that talks, and name the tiles a decode cannot check
- *(xray)* drive a pin in a `*_SING` IO tile, and say why when a ball cannot be
- *(ip)* an ISO 7816-3 character layer, so a card can be talked to
- *(fpga)* put the ECP5's carry chain on the fabric, and place it
- *(fpga)* a third carry shape, and the ECP5's CCU2C is it
- *(xray)* place, route and configure a RAMB18E1 with its contents
- *(examples)* a Basys 3 counter written as count + 1
- *(xray)* legalise a mapped CARRY4 chain for a 7-series slice
- *(xray)* give a slice's CARRY4 its pins, its muxes and its constants
- *(place)* read dedicated wiring off the graph and keep it rigid
- *(fpga)* a config entry for a pin tied to a constant
- *(xray)* place, route and configure a PLLE2_BASE on the 7-series fabric
- *(fpga)* let a design read its generated clock's LOCKED, and place a PLL
- *(xray)* place a RAM64X1D on a SLICEM's D and C lookup tables
- *(xc7)* map a multiply onto DSP48E1
- *(fpga)* let a `dsp` line tie pins, set parameters and multiply signed
- *(fpga)* say when a design outgrows its part number, not its die
- *(xray)* switch on a pad's weak pull-up when the constraints ask
- *(fpga)* an output a tri-state drives becomes the family's OBUFT
- *(xray)* route a 7-series tristate through the OLOGIC to the pad's T
- *(spi_display_rx)* state the rate as a ratio of clocks, not a frequency
- *(ip)* receive a display's four-wire SPI by oversampling it
- *(ip)* uart 1.1.0, because the character format is new ports
- *(uart)* let the host choose the framing, and say which error it was
- *(ip)* an inflate block, and the compcol corpus it will be checked against

### Fixed

- *(xray)* gate the clock-buffer test on the features its helper needs
- *(xray)* a `BUFGCTRL` is not a wire, and a clock walked through one for free
- *(xray)* `WEBWE[7:4]` is not used in TDP, so it may not fail a build
- *(docs)* the library footprints, three rows this round moved and two it did not
- *(fpga)* a constant on a block RAM's address or a pad's data, two faces
- *(basys3)* a monitor on the contact, and untrack six build artefacts
- *(docs)* retract "bad balls" — the count moves with the route, not the pin
- *(docs)* five footprint rows the table had not been regenerated for
- *(fpga)* a PLL has run on a part, and its own two tiles do not decode
- *(ip)* spi_display_rx counted a frame its chip select never closed
- *(xray)* a tileconn join must land on the tile type the file names
- *(docs)* retract two board findings; the fault is the clock
- *(tests)* an import the gate's per-feature clippy loop refuses
- *(fpga)* a constant on a carry cell's operand needs a driver, not a tie
- *(tests)* a cast nightly clippy refuses and stable does not yet
- *(xray)* refuse a DSP48E1 by name, and keep multiplies in LUTs for --bitstream
- *(program)* hand the FTDI channel back to the serial driver on macOS
- *(test)* two branches, two meanings of `Frame`, one file
- *(uart)* eight modules in four files, because a file list is an interface too

### Other

- *(ip)* the glitch filter costs one flip-flop, even switched off
- *(xray)* the A/B pair, and what ruled the pad out
- *(fpga)* the nine placements, measured rather than asserted
- *(xray)* simulate a wide status word's mapped netlist, and verify the 7-series routing
- *(fpga)* count the exemption rows correctly
- *(xray)* the address remedy is the idle policy, not a driver
- *(fpga)* say which remedy each of the two pins got
- *(fpga)* what two databases and three bitstreams settled about constants
- *(fpga)* no primitive input is left holding a constant, for every role
- *(xray)* a `*_SING` IO tile drives a pin on silicon
- *(xray)* an ISO 7816 card brought up on a Basys 3, and it answers
- *(xray)* six Pmod balls lose their decode when used bidirectionally
- *(xray)* the serial port works, and four subsystems check out on the part
- *(xray)* the carry chain is right, on an instrument checked first
- *(fpga)* the four designs counted, and which nets the welded joins hit
- *(xray)* most IO pads do not follow their logic on a Basys 3
- *(xray)* a carry chain on a Basys 3, and it computes the wrong number
- *(fpga)* the move-window experiment was run, and it is not the fix
- *(fpga)* the placer this fabric's chain actually goes through, re-measured
- *(place)* one dedicated-wiring mechanism, measured on two families
- *(ip)* re-measure the twelve rows the merge left stale
- *(fpga)* the gap this round leaves open is routing, and it is measurable
- *(fpga)* break a paragraph that two edits ran together
- *(fpga)* 57 tiles, because the pointers gained a share of two constants
- *(fpga)* what the crypto console measures now, and that routing got slower
- *(fpga)* two LUT4 counts the constant drivers moved
- *(fpga)* two board tests pass with carry chains in them
- *(ip)* put the iCE40 paragraph after the two it was interrupting
- *(fpga)* the iCE40 place, route and bits goldens the two constant LUTs move
- *(fpga)* the structural check the board had to stand in for
- *(fpga)* forty-four rows, not thirty-nine
- *(fpga)* place the carry chain twice and insist it lands the same
- *(ip)* inflate's Adler-32 is the smallest measurement of the gap
- *(fpga)* read a carry cell's truth tables back out of the image
- *(fpga)* the CCU2C, the direction of its chain, and the bit it shares
- *(fpga)* read 2131 CCU2C out of the vendor's bitstreams, and route one
- *(ip)* re-measure every ECP5 row, now that an adder is a carry chain
- *(fpga)* show a solver the inside of a CCU2C
- *(fpga)* the exploratory test that reads a CCU2C out of a vendor bitstream
- *(xray)* what block RAM rests on, checked and quoted
- *(xray)* the PLL, what is computed, what is quoted, and the reset trap
- *(xray)* a distributed RAM on a SLICEM, checked and quoted
- *(xray)* a distributed RAM to a decoded .bit, and a board demo for it
- *(xray)* what a 7-series tristate costs, and which parts are quoted
- *(xray)* a bidirectional Pmod pin and an OBUFT, decoded to the routing
- *(spi_display_rx)* hold every output to the registers-only walk
- *(ip)* what a block written from nobody's specification looks like
- *(uart)* put the UART's two handshakes under the in_ready rule
- *(uart)* what the board said, which is that five codings are five waveforms
- *(ip-library)* introduce the UART's page, and what a loopback cannot see
- *(uart)* the catalogue, the footprints and the block README
- *(ip)* check that a stream's `in_ready` really is a function of registers
- *(ip)* what inflate's tests do, and that compression is the half still missing
- *(ip)* the compress category, and what inflate measured
- *(ip)* measure inflate against compcol, and fix the byte a stalled copy dropped

## [0.0.5](https://github.com/KarpelesLab/reticle/compare/v0.0.4...v0.0.5) - 2026-10-07

### Added

- *(fpga)* build the console with either crypto core, because both do not route
- *(fpga)* both crypto blocks behind a Cynthion's USB serial port
- *(ip)* a crypto category, with SHA-256 and ChaCha20
- *(cli)* walk the IP library for `reticle build`, and --locked
- *(ip)* resolve a dependency by name against an IP library
- *(ip)* two pulses that say what the proxy forwarded, and the board's self-powered bit
- *(ip)* a USB transaction proxy behind the hub, with pass-through addressing
- *(trellis)* a DP16KD can be placed, and its bits are in three tiles
- *(place)* give the annealer VPR's adaptive schedule and a range limit
- *(hub)* a single-port USB 2.0 hub the kernel's own hub driver binds
- *(fpga)* `--place-effort`, and the measurement that says not to lower it
- *(fpga)* a design on the TARGET port's balls places, routes and decodes
- *(fpga)* the left and bottom edges of an ECP5 are described
- *(ip)* a USB full-speed host behind a ULPI transceiver
- *(fpga)* a distributed RAM spends its tile's LSR1, and the placer knows
- *(monitor)* the whole machine answers through the board's transceiver
- *(examples)* a 6502 monitor with its console on a USB serial port
- *(uart)* the host's baud rate reaches the divisor
- *(usb)* the endpoint buffers are arrays, and the byte multiplexer is gone

### Fixed

- *(fpga)* either terminator ends a line, and a digit counter that saturates
- *(place)* a clock buffer may only take a site its driver can reach
- *(tests)* give the example tests the library index their manifests need
- *(ip)* a SETUP that preempts a transaction the engine is still running
- *(tests)* three lints nightly clippy reports that stable does not yet
- *(trellis)* a memory's clock keeps the threshold, and an idle port needs one tie
- *(place)* keep the public docs out of the private items
- *(place)* one helper for the range limit's conversion to tiles
- *(sim)* a lookup table is unknown only when an unknown input can change it
- *(hub)* the hardware test raced the kernel's own hub driver
- *(hub)* the board console's character counter was a bit too wide
- *(fpga)* a public doc comment linked a private function
- *(fpga)* the TARGET host's report printed every label beside another item's value
- two lints a newer clippy reports and this machine's does not
- *(ip)* four defects the host found, and the model defect that hid two
- *(monitor)* the ACIA's clock-source bits belong to the host, not the 6502
- *(monitor)* the ACIA's clock source is one field, not half of one
- *(tests)* the lints the gate insists on, and a correct file count

### Other

- *(crypto)* what the board said, which is no
- *(crypto)* ask the part whether one line gets one answer, before asking whether it is right
- *(fpga)* what LED 5 actually says, which is two signals behaving differently
- *(crypto)* check the big keystream digests, and put the key back first
- *(crypto)* the console in simulation through the whole USB stack, and on a part
- *(ip)* re-measure the library index now that it holds thirty-one
- *(ip)* what each crypto block defends against, and what it does not
- keep the two-line `Pins:` / `Sources:` headers two lines
- *(ip)* what the move measured, and the lock file it writes
- rewrap the paragraphs the longer IP paths overflowed
- *(ip)* follow the packages into their categories
- *(ip)* pin the `library` grammar, and say why P0101 was looking there
- *(ip)* the library root, the index's measured cost, and the three failures
- *(fpga)* which eleven signals the proxy board design does not route
- the proxy measured on a part, and the CDC port re-verified after GET_STATUS
- *(ip)* the proxy's page, the library's, and the hub's reset and GET_STATUS
- *(fpga)* the block RAM's image is fifteen bits smaller on the new annealer
- *(trellis)* a block RAM's pins come out of the database, and the refusal is not shown failing
- *(trellis)* what ecppack writes for a block RAM, and a design that uses two
- *(place)* say what the start-temperature probe samples now
- *(place)* say which per-move numbers were counted and which were read
- *(place)* the lutram case, and the arc count that was really 693
- *(trellis)* pin the new placements, and stop pinning a placer's choice
- *(place)* say where the 73-tile window comes from
- *(place)* the account of the annealing schedule, measured
- *(sim)* what `x` costs, cell by cell, and where it is still invented
- one more section number that should be a named port state
- *(hub)* cite only the subsections whose numbering has been checked
- *(hub)* the board design's console names the fields its header claims
- *(hub)* the hub in the footprint table, the equivalence proof and the
- *(hub)* ten simulation tests of the hub, through both link layers
- *(fpga)* the routing goldens gain the line that counts the search
- *(fpga)* what an ECP5 build costs, and a guard that counts rather than times
- *(fpga)* two inner loops that did thirty-two times the work asked of them
- *(fpga)* the placer rescanned a thousand-sink net on every move
- *(fpga)* the ECP5 flow's fixed cost, and the placer's legality check
- *(synth)* the performance figures are the final ones, measured twice
- *(synth)* the profile after the work, beside the profile before it
- *(synth)* where synthesis spends its time, and two guards that count work
- *(synth)* the cofactor built a truth table to read one word out of it
- *(synth)* the cone walks allocated and hashed once per cut, and twice per proof
- *(synth)* the mapper spent half its time hashing four-byte keys
- *(tests)* two lints on the report test's byte capture
- the left edge of this die is on a part, and a transceiver's vendor ID is the witness
- *(trellis)* the section that quoted the error now quotes the bitstream
- *(fpga)* two doc comments that still counted two edges
- *(trellis)* all four edges of the pad model, and what a board cannot show here
- *(fpga)* what Lattice's own packer writes for a left- and a bottom-edge pad
- a green gate here is not a green CI, and the toolchain is why
- *(ip)* the device's answer delay is one number now, and the model is why
- *(ip)* a descriptor in packets of eight, and the bytes against a third implementation
- *(fpga)* two edges of the die, and the day the other two stopped being theoretical
- *(tests)* rustfmt the transceiver model's new guard
- *(ip)* the host's page, and what writing a second thing against one bus found
- *(monitor)* the rejection path and the editing keys, against the original
- the three examples pin uart's fourth source
- the example that ends on a part, in the roadmap
- *(monitor)* which of the ACIA's facts were read and which were inferred
- *(monitor)* the terminal session, and the one measurement nobody took
- *(monitor)* the ROM costs one lookup table a byte, measured
- the file every agent in this project has been told to read
- *(fpga)* the probe reaches eight lookup tables, not sixty-four
- *(usb)* quote the flow's own reason for the iCE40 fallback
- *(usb)* the shift-register shape moves bytes too
- *(usb)* the array's footprint, and the permutation that was never tested

## [0.0.4](https://github.com/KarpelesLab/reticle/compare/v0.0.3...v0.0.4) - 2026-09-28

### Added

- *(usb)* carry 64 bytes a packet and send SERIAL_STATE
- *(fpga)* model an ECP5 slice's distributed-RAM mode, so a design with one places
- *(synth)* prove a mapped netlist equivalent to what it was mapped from
- *(fpga)* a serial port on the Cynthion, bridged to a UART and looped back
- *(ir)* every vendor spelling of keep, and a keep that is off
- *(verilog)* the black-box warning says there is no search path
- *(asic)* the same check, with the liberty library as what is supplied
- *(fpga)* refuse a design with a hole in it before mapping any of it
- *(ir)* name the module an instance cannot find, and the instance
- *(fpga)* a constant zero is on a part now, in a byte a host reads back
- *(fpga)* the Cynthion's AUX port loops endpoint 1 back, and a host drives it
- *(ip)* endpoints beside endpoint 0, and descriptors the class writes
- *(fpga)* a flip-flop whose data is a constant gets a lookup table to take it from

### Fixed

- *(usb)* send SERIAL_STATE when the host opens the port, not once per configuration
- *(ip)* read the descriptor blob with one part-select again
- *(synth)* compose a merged cut's function instead of re-simulating its cone
- *(fpga)* the echo register is taken before it is refilled, not after
- *(test)* the reattach is not a collapsible if
- *(test)* factor the mapping test's cases into a struct, and the roadmap
- *(ip)* the descriptor ROM is read a page at a time, because LUT4 got it wrong
- *(synth)* the keep predicate is public, and the sim census knows keep_reg
- *(synth)* a keep on a register keeps the register
- *(ir)* the note describes a declared black box, it does not promise one
- *(sim)* a module nothing declares cannot be simulated
- *(ip)* the PLL wrapper passes on the whole block, not most of it

### Other

- *(usb)* say what was measured, not a model of it
- *(usb)* the notification test's list has five items, not four
- the placement gap these two documents call permanent is closed
- *(usb)* the throughput figures are in section 5, not section 6
- *(usb)* the last two places that said an open waits for a carrier
- *(roadmap)* correct the carrier claim and the distributed-RAM one
- *(usb)* give the throughput's spread, not only its median
- *(usb)* pin the boundary of the over-long packet check
- *(usb)* send SERIAL_STATE through the ULPI transceiver too
- *(ip)* one round-trip figure disagreed with the prose beside it
- *(fpga)* the constant-zero probe reaches eight lookup tables, not sixty-four
- *(usb)* the terminal wait is five seconds, not one
- *(usb)* the manifest and the block table say the notification endpoint sends
- *(usb)* the test file's header explains the transceiver model's correction
- *(ip)* the library page's CHECKED list and test summary name the notification
- *(usb)* re-quote section 5 off the part, with the notification in it
- *(usb)* name the distributed RAM as the shape to try next
- *(ip)* refresh the footprint table after the bus-reset guard
- *(usb)* cite sections of PSTN 1.2 rather than table numbers not checked
- *(usb)* the roadmap and the READMEs say 64 bytes and a notification
- *(usb)* what the packet size costs and what it bought
- *(usb)* hold the board's designs and tests to 64-byte packets
- *(usb)* measure the loopback's throughput on the part
- *(fpga)* pin the distributed RAM's wire table and its address permutation
- *(fpga)* write down everything ecppack writes for a distributed RAM
- *(roadmap)* record mapped-netlist verification under phase 5
- the flow's own output is not a file to commit
- *(ip)* mark the driver's reasons as readings, and re-measure the bitstream
- *(ip)* three measurements of the FIFO refusal, not five
- *(ip)* say which ECP5 the FIFO was refused on, and which is inferred
- *(ip)* eight-byte packets cost transactions, not the number that was quoted
- *(cli)* read the line coding back off the part, and mark the claims honestly
- *(ip)* the serial port, its provenance, and what a host said
- *(cli)* the kernel's own driver binds it, and bytes go round through a UART
- *(ip)* the serial port enumerated, its requests answered, its bytes moved
- *(viewer)* the site for the new synthesis golden
- *(synth)* name the two places a keep is still dropped in silence
- *(synth)* the backend packers are not as careful as the optimiser
- *(synth)* say exactly which rewrites a kept object still gets
- what keep promises, and what still applies to a kept object
- *(synth)* keep from both front ends, through the whole pipeline
- *(ip)* the silent black box that cost a bogus FAIL is a diagnostic now
- where the line between a black box and a missing module falls
- *(cli)* the Cynthion build without its IP sources names the module
- *(fpga)* say that the control bitstream was a scratch copy and was put back
- *(fpga)* the probe's header names the test that exists, and the right cost
- *(fpga)* the unused reset wire is a different shape, and it is measured
- *(fpga)* a constant one is on a part now, and a constant zero is not
- the footprints both of this week's changes add up to
- what the endpoints are, what is derived, and what has run on a part
- *(ip)* a byte index is a shift, and this compiler does not know that yet
- *(ip)* bytes through endpoint 1, through both link layers and both ways
- the constant is built now, and the vendor's own bitstreams said how
- *(fpga)* every flip-flop of Lattice's own bitstreams has a driver, and four are constants

## [0.0.3](https://github.com/KarpelesLab/reticle/compare/v0.0.2...v0.0.3) - 2026-09-27

### Added

- *(fpga)* the console and the ULPI trace that found the fault
- *(fpga)* the edge rate every ULPI pin of this board asks for
- *(fpga)* a Cynthion told that its own board crosses DP and DM
- *(fpga)* a design that puts a Cynthion's transceiver back to sleep
- *(fpga)* a USB device on a Cynthion's AUX port, built and loaded
- *(fpga)* an eight-bit bidirectional bus on a Cynthion's right edge
- *(fpga)* a field is read back as the value that leaves fewest bits unexplained
- *(fpga)* the bidirectional pads of a Lattice ECP5
- *(verilog)* a conditional assignment to high impedance is a tri-state driver
- *(fpga)* an inout port with a tri-state driver becomes a bidirectional pad
- *(ip)* a USB device over ULPI, for a board whose lines the FPGA cannot drive
- *(fpga)* the global clock network of the Lattice ECP5
- *(fpga)* a base cost per node, so a wire class can be preferred
- *(fpga)* interconnect for the Lattice ECP5, and a routed design on a board
- *(program)* configure a Lattice ECP5's SRAM over a Cynthion's Apollo
- *(fpga)* a Lattice ECP5 fabric from Project Trellis, and its pads
- *(vhdl)* evaluate at elaboration what a width can be made of
- *(program)* read an ECP5 through a Cynthion's Apollo debugger
- *(fpga)* per-bank IO standards for Gowin, and a four-button test
- *(fpga)* write and load a Gowin GW2A bitstream, and run one on a board
- *(cli)* fetch the chip databases into ~/.cache/reticle
- *(fpga)* describe the GW2A-18 of a Sipeed Tang Primer 20K
- *(fpga)* read Project Apicula's Gowin chip database as an Arch
- *(ir)* let a `FileProvider` hand over bytes as well as text
- *(fpga)* the Gowin `.fs` bitstream container
- *(msgpack)* a hand-written MessagePack reader
- *(fpga)* route a clocked design across a real 7-series clock tree
- *(fpga)* map a clocked design onto real 7-series sites
- *(fpga)* give a real 7-series bel its pins, and route the milestone
- *(cli)* `reticle program <file.bit>` loads a bitstream into a board
- *(program)* configure a 7-series FPGA over JTAG, sans-I/O
- *(fpga)* emit a 7-series bitstream from `reticle fpga`
- *(fpga)* read a real 7-series fabric and write a real .bit
- *(examples)* give the NES a Basys 3 target through vga_out
- *(cli)* override a parameter or generic from the command line
- *(fpga)* map 7-series adders onto CARRY4
- *(examples)* give the Apple II a Basys 3 target through vga_out
- *(examples)* add an NES-compatible console built from the IP library
- *(examples)* add an Apple II-compatible computer with DVI video
- *(test)* give the 6502 assembler the `<` and `>` byte operators
- *(fpga)* take a 7-series design to the files Vivado reads
- *(fpga)* describe the Xilinx 7-series and the XC7A35T

### Fixed

- *(fpga)* a slew rate a tile type does not declare is not a missing pad
- *(ip)* four states in two bits, and a host reads the descriptor
- *(fpga)* a register bit nothing drives is refused, not brought up as a one
- *(ip)* a turnaround a host would wait for, not only one ULPI asks for
- *(ip)* three things a transceiver's datasheet says that ULPI does not
- *(ip)* a pair whose pull-up is still charging is not a bus reset
- *(program)* a bitstream's compression dictionary is not a header command
- *(fpga)* two cells that share a bel pin must agree about what is on it
- *(ir)* report a reversed slice instead of panicking
- *(vhdl)* lower a clocked conditional assignment, and name the latch
- *(vhdl)* six defects semantic analysis showed on real VHDL
- *(vhdl)* parse the attributes and ranges the grammar spells oddly
- *(cli)* list every adapter, not only the FTDI ones
- *(program)* report a missing Cynthion as one, not as a missing cable
- *(fpga)* give each bit of a bus constrained bit by bit its own pin
- *(program)* account for every bit of the status word, reserved included
- *(program)* stop shifting Xilinx instructions at a part that is not one
- *(fpga)* record the GW2A-18's speed grade as the string it is
- *(program)* read the adapter's chip and the part's IDCODE without guessing
- *(test)* link the C example against macOS's USB framework
- *(fpga)* use a frame address the tiny test part really has
- *(test)* brace the names the NES's Vivado script asserts
- *(test)* name the one-bit waveform the video decoder takes
- *(test)* keep the carry tests compiling without the formal feature
- *(verilog)* accept a string literal where a vector is wanted
- *(test)* compare the lowering tripwire against itself, not a clock
- *(fpga)* stop unique_name rescanning the module per candidate
- *(synth)* collect dead expressions before renumbering memories
- *(fpga)* stop two false constraint reports on a hierarchical design

### Other

- one flip-flop fewer, because a state register is two bits wide
- *(ip)* a transceiver that reports LineState late, which this one does
- *(ip)* the falling turnaround is the link's, so the model demands it
- the board crosses D+ and D-, and the host now calls it full speed
- *(fpga)* the Cynthion top level's transceiver starts its pair at SE0
- the transceiver and the host name opposite wires
- a transceiver's registers outlive the FPGA's configuration
- correct three measurements, and say where the variants went
- an eight-bit bidirectional bus, and a host that saw a device attach
- the bidirectional pad reads back what it drives
- a bidirectional pad has been loaded into the Cynthion's ECP5
- *(test)* the USB host model drives a pair, not a device's pins
- the Cynthion's clocked design blinks
- a clocked design has been loaded into the Cynthion's ECP5
- a routed design works on the Cynthion's ECP5
- *(fpga)* the Cynthion does not number its LEDs, so say which end to read
- *(fpga)* stop allocating a wire's name for every pip of a graph
- the Cynthion's LEDs are lit
- *(vhdl)* measure the front end against CERN's Colibri library
- *(program)* drop a redundant explicit intra-doc link target
- *(apollo)* say how many times the read was performed, and from where
- *(roadmap)* record the Apollo transport and the ECP5 it identified
- *(apollo)* record what the board confirmed and what it contradicted
- *(apollo)* specify the Cynthion debugger's USB protocol
- *(fpga)* say that the Basys 3 blink design was watched blinking
- *(fpga)* drop a block left over from collapsing an if in the loader
- *(fpga)* say that `--probe` is not safe to point at a Gowin part
- what the Gowin flow reads, and what it has never done
- what the clocked design established on a real part, and what it did not
- say that one design from this flow has run on a part
- *(fpga)* intern a pip's configuration bits so a real die fits
- *(program)* what loading a bitstream into a real board established
- *(fpga)* say what the real 7-series flow establishes and what it does not
- *(json)* share the JSON parser between the LSP and the FPGA side
- *(examples)* document the NES's Basys 3 target and its colour depth
- *(fpga)* say what the carry report counts on a wide element
- *(fpga)* record the 7-series carry chain and what it saves
- *(examples)* count the fourth package and the fourth source
- *(examples)* map the Apple II onto the Basys 3's Artix-7
- *(ip)* hold vga_out to the raster, the blanking and the polarity
- tidy the example list after merging two branches into it
- *(nes)* check the $2007 data port and what $2000 does to `t`
- record examples/apple2 in the README and the roadmap
- tidy the two lines the 7-series note added
- *(fpga)* map a small memory onto the 7-series distributed RAM
- describe the Xilinx 7-series backend and what it does not prove
- *(fpga)* take three designs onto the Artix-7 end to end

## [0.0.2](https://github.com/KarpelesLab/reticle/compare/v0.0.1...v0.0.2) - 2026-09-22

### Added

- *(examples)* add a MOS 6502 computer built from the IP library
- *(ip)* add a MOS 6502 core to the library

### Fixed

- *(test)* stop the language server goldens depending on the version
- *(test)* stop the FST goldens depending on the crate version

### Other

- add a guide to packaging a CPU as Reticle IP
- *(test)* share the 8N1 waveform decoder between the examples
- *(verilog)* pin the formatter moving a parameter's comment

## [0.0.1](https://github.com/KarpelesLab/reticle/compare/v0.0.0...v0.0.1) - 2026-09-22

### Added

- *(formal)* decide combinational miters by SAT sweeping
- *(cli)* give every synthesising command a file provider
- *(fpga)* describe block RAM layouts and partial pin lists in the device database
- *(ip)* add usb_device_fs, a full-speed USB device that enumerates
- *(ip)* add eth_mac_rgmii on eth_mac_rmii's frame logic
- *(ip)* add dvi_tx, DVI output serialised through DDR registers
- *(ip)* add hyperram_ctrl, a HyperBus controller through the DDR IO path
- *(ip)* add sdram_ctrl, an SDR SDRAM controller held to the datasheet
- *(cli)* add lsp, search, add, asic and an interactive sim
- *(fpga)* configure double-data-rate IO registers and IO delays
- *(fpga)* instantiate a PLL for a clock the board does not have
- *(fpga)* duplicate a block RAM for a register file's read ports
- *(fpga)* build a memory that misses a block RAM out of logic
- *(fpga)* invert a reset the family has no polarity for
- *(synth)* add arithmetic lowering with a choice of architectures
- *(viewer)* render a design as schematic and reference HTML pages
- *(cli)* add `reticle cache` over a directory-backed store
- *(cache)* add a content-addressed cache of elaborated modules
- *(ip)* add rv32i, eth_mac_rmii and spiflash_xip to the library
- *(sim)* add a compiled two-state cycle-based fast mode
- *(ip)* add a static registry index and the library half of `reticle add`
- *(ip)* import IP-XACT component descriptions
- *(asic)* map designs onto a Liberty library and hand off to OpenROAD
- *(ffi)* add a C API and a WebAssembly surface for embedding
- *(synth)* report estimated combinational depth
- *(fpga)* add placement, routing and bitstream generation
- *(lsp)* add a language server for Verilog and VHDL
- *(cli)* check assertions and write coverage from sim
- *(sim)* add an interactive session with breakpoints on net changes
- *(sim)* add line and toggle coverage with a text and LCOV report
- *(sim)* add concurrent assertions with an SVA and PSL subset
- *(ip)* add the Reticle IP library blocks under ip/
- *(vhdl)* bundle the remaining ieee libraries with native builtin bodies
- *(cli)* add the build command for IP projects
- *(ip)* lower VHDL sources in a project build
- *(ip)* add IP and project manifests with dependency resolution
- *(cli)* add the timing command
- *(timing)* add clock domain crossing analysis
- *(timing)* add static timing analysis
- *(vhdl)* add elaboration and lowering to the IR
- *(cli)* add the fpga command and a synth equivalence flag
- *(fpga)* map to device primitives and complete the nextpnr flow
- *(synth)* add post-synthesis equivalence checking
- *(synth)* add the cellify pass
- *(cli)* expose technology mapping from synth
- *(fpga)* add device database, primitive mapping and constraints
- *(synth)* add AIG optimiser and LUT/gate technology mapping
- *(cli)* take Verilog sources for every stage, add fmt
- *(verilog)* add elaboration and lowering to the IR
- *(vhdl)* add semantic analysis and the std/ieee libraries
- *(vhdl)* add the source formatter
- *(verilog)* add the source formatter
- *(fmt)* add a Wadler document printer and a line diff
- *(cli)* write FST waveforms from sim
- *(sim)* add FST waveform writer with in-crate LZ4 and zlib
- *(ir)* add flattening, uniquification and hierarchy queries
- *(verilog)* add AST-level linter with 28 rules
- *(asic)* add Liberty, LEF and DEF readers and writers
- *(cli)* wire check, synth, emit, sim and verify subcommands
- *(synth)* add process lowering, FF/latch/memory/FSM inference and opt passes
- *(ir)* add Verilog, VHDL, JSON, BLIF and EDIF emitters
- *(formal)* add bit-blaster, BMC, k-induction, equivalence checking and reachability lint
- *(sim)* add event-driven simulator, VCD writer and cosim API
- *(verilog)* add parser and AST
- *(vhdl)* add the VHDL-2008 parser and AST
- *(formal)* add CDCL SAT solver and Tseitin encoder
- *(ir)* add the unified design IR with text format
- *(verilog)* add preprocessor and lexer
- add string interner and 4-state Logic value type
- *(vhdl)* add the VHDL-2008 lexer

### Fixed

- *(cache)* record the files synthesis reads and re-check them on every hit
- *(verilog,sim,synth)* load memory files, run bare system tasks, print memory words
- *(fpga)* flatten the design in synthesize_for
- *(fpga)* give block RAM its initial contents and duplicate read-only memories
- *(cli)* let sim read $readmemh files from disk
- *(ip,fpga)* keep a project's top, and give a zero-step delay nothing
- *(asic)* round the area in the flow report
- *(test)* skip the C link test on the MSVC target
- *(synth)* let the gate mapper invert a gate's output
- *(viewer)* keep a bus width when its wire label has to be cut
- *(viewer)* cut a wire label to the room before the next box
- *(synth,fpga)* two defects writing the RISC-V core exposed
- *(sim)* close eight divergences a code review found in compiled mode
- *(sim)* apply chained asynchronous resets in dependency order
- *(sim)* give every compiled operation kind its own cache key
- *(ip)* word the inferred bus prefix note for the case with no prefix
- *(sim)* compute the gzip trailer length in 64-bit arithmetic
- *(timing)* recognise a synchroniser written as a shifting register
- *(timing)* say when a netlist's storage is hidden in black boxes
- *(synth)* keep an asynchronous reset asynchronous after a VHDL frontend
- *(vhdl)* analyse component instantiations and defaulted generics
- make the cli feature build and pin golden line endings
- *(test)* skip environment-dependent tests instead of failing
- *(test)* drop a duplicated feature guard in the lint test

### Other

- *(synth)* share the FRAIG simulation classes and sweep loop
- *(soc)* assert the four FPGA backend fixes, and update the docs
- *(ip)* prove phase 8 on a RISC-V SoC built from the IP library
- *(test)* move the RV32I assembler into a shared test module
- mark the device-primitive IP blocks done in the roadmap
- *(fpga)* pin a zero-step IO delay still building a delay element
- mark arithmetic lowering done in the roadmap
- *(synth)* prove and measure the arithmetic architectures
- *(cache)* write up the incremental build, measurements included
- *(cache)* cover the invalidation directions and a damaged store
- record the larger IP blocks in the roadmap and changelog
- *(sim)* re-measure compiled fast mode after the correctness fixes
- *(sim)* report the compiled fast mode measurement properly
- *(ip)* document the registry index and the IP-XACT import
- note the ASIC standard-cell flow in the README
- *(asic)* document the ASIC flow and tick phase 6
- note the C API, WebAssembly build and language server
- *(ffi)* pass --lib in the documented cargo rustc commands
- *(fpga)* document the routing architecture and tick phase 6
- *(sim)* add a simulation guide covering the whole stage
- *(ip)* point at the IP library and tick phase 8's library item
- *(ip)* co-simulate and measure every IP library block
- note the bundled ieee packages in the README
- *(vhdl)* record where the numeric lowering departs from the packages
- *(ip)* drop the unused requirement table from the lock builder
- *(ip)* document the manifest formats and tick phase 8
- *(ip)* add golden project builds and example IP packages
- *(timing)* document the timing model and what it leaves out
- *(timing)* add golden timing and crossing reports
- describe the source formatters
- *(sim)* mention FST capture and guard its test by feature
- *(ir)* point walk's module docs at the new hier module
- guard integration tests by feature and widen the matrix
- *(ir)* use logic::Logic as the IR constant type

### Added

- IP library: three larger blocks under `ip/`, taking it to fourteen.
  `rv32i` is the whole RV32I base integer instruction set in a
  multi-cycle machine-mode core with traps, interrupts and the machine
  CSRs, tested by assembling RISC-V machine code and running it — every
  instruction class, an array summed in a loop, and Fibonacci computed
  recursively on a stack. `eth_mac_rmii` is an Ethernet MAC over RMII,
  which is single data rate and so needs no device primitive the FPGA
  backend lacks, tested by looping its transmitter into its receiver and
  by rejecting a frame with a flipped dibit. `spiflash_xip` is a
  read-only execute-in-place path from a serial NOR flash, presenting
  the same memory port `rv32i` puts on its instruction side. Each has a
  manifest, a co-simulation test and a measured footprint in
  `docs/ip-library.md`.
- Incremental compilation behind the new `cache` feature: a
  content-addressed store of elaborated and synthesised modules, keyed on
  the source text that produced them, the options, the parameter set, the
  compiler version, the feature set and the keys of their dependencies.
  Editing a leaf invalidates it and everything above it; editing a
  top-level file invalidates only that module. Artefacts are the IR's
  `.rtl` text, so an entry is inspectable by hand. `Storage` is a
  four-method trait — the library ships the in-memory backend and the
  `reticle cache` command supplies a directory-backed one — with eviction
  by total size in least-recently-used order and a `verify` that catches a
  store damaged behind the build's back. See `docs/cache.md`, which
  includes the cases where the cache does not help.

- VHDL: the remaining bundled standard libraries — `ieee.numeric_std`,
  `ieee.numeric_bit`, `ieee.math_real`, `ieee.std_logic_textio` and the
  Synopsys `std_logic_arith`, `std_logic_unsigned` and `std_logic_signed`.
  Each ships its declarations as VHDL and marks every subprogram
  `attribute foreign`; the bodies are native Rust over `logic::Logic`,
  shared between the analyser's constant folding and the elaborator's
  lowering to IR operators. A design using `unsigned` or `signed`
  arithmetic now analyses, elaborates and simulates.

### Fixed

- Verilog `$readmemh`, `$readmemb`, `$writememh` and `$writememb`, with
  their optional start and end addresses. The IR has a statement for
  them, `StmtKind::MemFile`, that names the memory itself; the Verilog
  frontend used to pass the memory as a string, which the simulator
  refused, so no `$readmemh` written in Verilog loaded anything. The
  simulator now loads through its `FileProvider` and hands saved files
  back through `Simulator::written_files`; synthesis reads an
  `initial` block's `$readmemh` through the same trait, given in the new
  `SynthOptions::files`, into the memory's initial contents, and says
  which file it could not load (`S0018`) instead of dropping the call.
  The trait and `MemoryFiles` moved to `ir::memfile` and are still
  re-exported from `sim`.
- Verilog: a system task written without parentheses (`$finish;`,
  `$stop;`, `$display;`) is the same call as its parenthesised form; it
  used to be dropped without a word.
- Verilog: a memory element in a `$display`-family argument
  (`$display("%h", mem[1])`) prints the word, not the memory's name.
