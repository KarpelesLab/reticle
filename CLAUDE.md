# Working in this repository

`CONTRIBUTING.md` has the rules every change must follow — no third-party
crates, sans-I/O, diagnostics not panics, Conventional Commits. Read it first;
this file does not repeat it.

What follows is the part that is not obvious from the code, and most of it was
learned by getting it wrong.

## The gate

```sh
{ tools/check.sh; echo "CHECKEXIT=$?"; }
```

**Judge it by that number and never by the log tail.** A run can print
hundreds of passing tests and still fail: a red gate was once pushed because
its tail looked green. `tools/check.sh quick` skips packaging and the MSRV
check when you are iterating.

**Its last step refuses a dirty working tree**, so commit before gating or you
get `CHECKEXIT=101` with every test passing. The message says
`N files in the working directory contain changes that were not yet committed`.

**A gate measured on a base that has since moved does not count.** If `master`
gains commits under your branch, rebase and gate again — especially after a
release, because a version bump is exactly the kind of change that breaks a
golden.

**A green gate here does not mean a green CI, and lints are why.** There is no
`rust-toolchain` pin, so CI tracks latest stable while this machine's toolchain
is whatever was installed — which has been months behind. A clippy lint added
upstream turns CI red on code that has not changed and that the local gate
passes: it happened on 2 October 2026, five `needless_borrow` errors in
`vhdl::sema::builtin`, on a commit that had passed CI two days earlier, because
stable went 1.98.0 → 1.99.0 on 28 September. Before pushing anything that
touches Rust, check with the newest clippy available —
`cargo +nightly clippy --all-features --all-targets -- -D warnings` is *ahead*
of CI rather than behind it, so a clean result there is worth something that a
clean result from an old stable is not. Doing that found a second lint
(`needless_range_loop`) that 1.99 does not yet report, which would have been the
next bump's red build.

**And clippy is not the gate.** `cargo fmt --check` runs before it. Removing two
characters to satisfy a lint changed how rustfmt wanted an expression wrapped
and failed the gate at its first step, with `CHECKEXIT=1` and zero tests run.
Verifying one step is not verifying the gate; run the whole thing and read the
number.

## Verifying claims

This project's method is to ask **"what does the vendor's own tool write, in
full, for this cell?"** rather than "what differs from what we emit". A diff
only disagrees about what you already produce. Reading what a vendor actually
wrote has found every real backend defect here, including one where the model
would have compiled, placed, routed and produced a bitstream in which every
bit decodes — and computed nothing.

Two checks that must never be weakened:

- **Every bit decodes.** A bitstream's set bits must all decode back through
  the database, and the arcs they select must be exactly the arcs the router
  chose. Report the count and that nothing is unexplained.
- **Mapped-netlist equivalence.**
  `every_block_maps_to_the_logic_it_was_mapped_from` proves every library
  block equivalent to what it was mapped from, at two lookup-table widths.
  It exists because the technology mapper once emitted wrong logic that
  only a person with an oscilloscope could have found.

**Documents here distinguish what was *checked* from what was only *quoted*,
and say which.** A specification reference is a reading until something
measured it. When a document's claim turns out to be wrong, correct it and
leave the wrong reading visible with its reason — several sections exist
precisely because the path to the answer is the useful part.

## Tests

- **A test broken by a legitimate change gets rewritten to pin the new
  behaviour, never deleted.** A test asserting the old behaviour may have been
  pinning a bug: one asserted that a design instantiating a module nothing
  declared elaborated and ran happily.
- **Say what a test would and would not catch.** A test that passes against a
  broken implementation is worse than none. If a new test would not have caught
  the bug it accompanies, say so — that has been the right answer more than
  once.
- **No wall-clock assertions.** CI runs on slower and Windows machines. Compare
  two runs in one process, or count work; never assert a number of seconds. A
  throughput figure is printed by an `#[ignore]`d test, never asserted.
- **Everything skips cleanly** when a fabric database or a board is absent, and
  prints why. Hardware tests are `#[ignore]`d.

## Fabric databases

Never committed — they are other people's build output. Fetched with
`reticle fetch <name>` into `~/.cache/reticle/<name>/<commit hash>/`.

`RETICLE_TRELLISDB`, if set, names the database **itself** — the directory
holding `ECP5/LFE5U-12F/tilegrid.json`, which is the commit-hash directory
*inside* the one the fetch created, not that one. Leaving it unset finds the
cached copy on its own, which is the easier way to be right. Likewise
`RETICLE_CHIPDB` (Project X-Ray, Xilinx) and `RETICLE_GOWINDB` (Apicula).

`RETICLE_ECP5_REF` points at a directory of Lattice reference bitstreams
(`analyzer.bit`, `selftest.bit`, `facedancer.bit` from the `cynthion` Python
package). Several tests cross-check against them and skip with a reason
otherwise — so **a green gate may have skipped them**. Set it when a change
touches the ECP5 backend.

## Building a design

**`reticle fpga` has no library search path.** It takes a list of files, and
a design instantiating library IP must name its sources. (`reticle build` is
the other way round: a project manifest's `library ../../ip` line plus
`depends uart ^1.0.0` finds the package by name, and `docs/ip.md` says how.
`fpga` stays a file list on purpose — it is given HDL, not a project.)

```sh
reticle fpga testdata/fpga/cynthion/usb_ulpi_device.v \
    ip/usb/usb_device_ulpi/rtl/usb_ulpi_link.v \
    ip/usb/usb_device_ulpi/rtl/usb_device_ulpi.v \
    ip/usb/usb_device_fs/rtl/usb_ctrl_ep.v \
    --device ecp5-12f-CABGA256 \
    --constraints testdata/fpga/cynthion/usb_ulpi_device.rcf \
    --bitstream /tmp/usb_ulpi_device.bit
```

A module nothing declares is an error (`I0034`) naming the module and the
instantiation. It used to become a silent black box whose undefined outputs
were reported against the *top level*, which misled two rounds of work.

With no `--output-dir`, the flow writes `<top>.json` and `<top>.lpf` into the
working directory; `.gitignore` covers them at the repository root only.

## Hardware

A Cynthion (Lattice ECP5 `LFE5U-12F`) is usually attached, reached through its
Apollo debug microcontroller: `reticle program --list` finds it.

- **`docs/apollo-protocol.md` §7 lists requests that must never be sent.**
- Write nothing non-volatile; enable no VBUS switch. Programming touches only
  the volatile configuration SRAM, and a power cycle reloads the board's flash.
- **Leave a working device loaded.** A broken one fills the user's kernel log
  and they see it.
- **One agent at a time.** Two programmers on one JTAG chain collide, and so do
  two `cargo` builds in one worktree.

A Basys 3 is sometimes attached too, with a smart card and its display
wired to it. **`docs/card-bench.md` is what to read before touching that**,
and the one rule that matters most: ball `N2` switches the device's 3.3 V
supply through a PhotoMOS relay, so it must be low at power-up, only the
activation sequence may raise it, and `D` goes at the end of every run.
Do not power-cycle somebody's device repeatedly without being asked.

Its console is **2.000 Mbaud**, not 115200.

`docs/fpga-trellis.md` is the account of this backend, including the faults it
has had and how each was found. It is worth reading before changing the ECP5
flow.

## Fabric hazards worth knowing

- **An unrouted slice input on an ECP5 reads as a one.** A flip-flop whose data
  input is a constant therefore used to come up holding a one. That cost eight
  rounds of investigation on a USB device that would not enumerate, because
  `reg [2:0] stage` for four states read **5** and every `case` label missed.
  The backend builds the constant properly now, but **never declare a register
  wider than the values it holds** — the habit is still right, and the same
  mistake has been caught twice more since in new code.
- **Simulation cannot see this class of defect at all**, because simulation has
  no unrouted wires. What found it was instrumentation on the part: a serial
  console on a spare pin and a trace buffer in the fabric, both committed under
  `testdata/fpga/cynthion/`.

## Integrating work

**Rebase, never merge.** `master` stays linear: rebase, gate, then push as
separate steps — never chain a rebase and a push in one command, because the
thing you push must be the thing you gated.

Watch the GitHub run after pushing. The local gate is Linux-only, and macOS and
Windows have each caught something it could not.
