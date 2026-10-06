# The annealing schedule

`docs/fpga.md` says what the placer is. This is the account of its
**annealing schedule**: what it was, why it improved nothing at all, what
the literature says a schedule should be, and what each piece of that was
worth when it was measured.

Everything below was measured on one machine in one sitting, with
`reticle fpga --timing --report`, so the seconds are comparable with each
other and with nothing else. The counted quantities — moves, pips,
wirelength, router nodes visited — are the same on every machine and are
what the tests assert on.

## The finding: it improved nothing

The placer's annealer did not work. Not "worked less well than it might":
it ended at **exactly** the wirelength legalisation handed it.

| design | moves tried | wirelength, before → after |
|---|---|---|
| `clock_blink.v`, effort 10 | 470 591 | 240 → **240** |
| `usb_host_target.v`, effort 1 | 6 864 042 | 28699 → **28699** |
| `usb_host_target.v`, effort 3 | 20 797 269 | 28699 → **28699** |
| `usb_host_target.v`, effort 10 | 70 693 890 | 28699 → 23770 |

Only the last row did anything, and it is the one that costs a hundred
seconds. The placer kept the best placement it ever saw, so a run that
"improves nothing" is a run whose random walk **never once got below
where it started**, in half a million moves on a hundred-cell design.

An earlier round saw the `usb_host_target.v` rows and read them as a
problem with the number of moves — below some budget the walk cannot
recover from its start temperature. The `clock_blink.v` row, measured
here, rules that out: 470 591 moves on a design of 106 cells is some five
thousand moves per cell, and it still never got back.

## What it was

The schedule had three of the four parts of Betz and Rose's, and the
missing one is the one that matters.

- **Start temperature** `20 × σ` of the cost change over a random move
  sequence. Present, but `σ` was taken over at most **100** samples
  whatever the design — a hundred draws from the distribution whose own
  spread is the quantity being estimated.
- **`effort · n^(4/3)` moves per temperature.** Present, with
  `effort = 10` where VPR's default is 1.
- **Cooling.** A fixed `T ← 0.9 T`, a hundred and twenty times, whatever
  happened.
- **A range limit on the move generator.** *Absent.* A move drew its
  destination uniformly from **every site of its kind on the die**.

## What the literature says

Betz and Rose, *VPR: a new packing, placement and routing tool for FPGA
research*, FPL 1997, §3. Quoted rather than paraphrased, because three of
these are numbers and the fourth is a table:

> We first create a random placement of the circuit. Next we perform
> `N_blocks` moves (pairwise swaps) of logic blocks or IO pads, and
> compute the standard deviation of the cost of these `N_blocks`
> different configurations. The initial temperature is set to 20 times
> this standard deviation, ensuring that initially virtually any move is
> accepted at the start of the anneal.

> the default number of moves evaluated at each temperature is
> `10 (N_blocks)^1.33`

> A new temperature is computed as `T_new = α T_old`, where the value of
> `α` depends on the fraction of attempted moves that were accepted
> (`R_accept`) at `T_old`

| `R_accept` | `α` |
|---|---|
| `> 0.96` | 0.5 |
| `0.8 < R ≤ 0.96` | 0.9 |
| `0.15 < R ≤ 0.8` | 0.95 |
| `≤ 0.15` | 0.8 |

> it is desirable to keep `R_accept` near 0.44 for as long as possible.
> We accomplish this by using the value of `R_accept` to control a range
> limiter -- only interchanges of blocks that are less than or equal to
> `D_limit` units apart in the x and y directions are attempted. …
> Initially, `D_limit` is set to the entire chip. Whenever the
> temperature is reduced, the value of `D_limit` is updated according to
> `D_limit_new = D_limit_old (1 − 0.44 + R_accept_old)`, and then clamped
> to the range `1 ≤ D_limit ≤` maximum FPGA dimension.

> the anneal is terminated when `T < 0.005 * Cost / N_nets`.

Two things follow that this placer had got wrong, and a third that the
paper could not have got wrong because its situation is not ours.

## Why a die-wide move set cannot work

The cooling table and the range limiter are **one mechanism in two
halves**, and the half this placer had was the passive one. `α` reacts to
the acceptance rate; only `D_limit` *acts* on it.

Without `D_limit`, a move on an ECP5 `LFE5U-12F` sends a cell to a random
one of 24 288 lookup-table sites on a die whose longest side is 73 tiles
— which is where the 73 in the window column below comes from, the
limiter's upper clamp being the largest die dimension. The wirelength
change such a move makes is tens of tiles. There is therefore **no temperature
at which such a move is useful**:

- warm enough to accept one, and the acceptance rate sits near 1, the
  walk is a random-placement generator, and the analytic solve's answer
  is gone;
- cool enough to be selective, and *every* move is a large uphill one and
  *every* move is refused. The budget is spent being told no.

The schedule passes through the second regime on its way down and never
visits a regime in between, because there is not one. That is the whole
explanation of the table at the top, and it is why raising the move count
looked like it helped: at effort 10 there are simply enough moves in the
brief window where the temperature and the die's scale overlap.

The measurement, `clock_blink.v` under the old schedule, one line per
temperature (`--place-schedule`):

```
  step   window     tried  accepted  rate   cost
     0       73      4570      4525   99%    5946
    10       73      4568      4490   98%    5456
    40       73      4569      3130   68%    4793
    60       73      4570      1138   24%    3018
    69       73      4569       599   13%    2113
```

It starts at a wirelength of 240, is at **5946** after one temperature,
and is still at 2113 when it gives up. The window column never moves,
because there was no window.

## What this placer is not

VPR anneals a **random** placement. This placer anneals the output of an
analytic solve and a legaliser, which have already put every cell roughly
where it belongs. Both of the paper's "start hot and start wide" choices
are chosen *so that* the initial placement is destroyed — "ensuring that
initially virtually any move is accepted" — and that is exactly the wrong
thing to do to a placement that is worth keeping.

So two of the four parts are adapted, and the adaptation is the part of
this that is not in the literature:

- **The start temperature is solved for, not scaled.** A bisection on the
  placer's own acceptance rule over the sampled cost changes finds the
  temperature at which the target fraction — 0.44, the same number the
  range limiter aims at — would be accepted. The walk begins where the
  schedule is trying to hold it instead of far above it.
- **The window starts at one tile**, not at the die. The limiter widens
  it, immediately and on its own, if the acceptance rate asks: that is
  what `D_limit_new = D_limit_old (1 − 0.44 + R_accept)` does when
  `R_accept > 0.44`.

Both of the paper's rules are still reachable — `--place-hot-start`,
`--place-start-window 0`, `--place-wide-moves`, `--place-fixed-cooling
0.9` — because the difference between a design decision and a preference
is whether it was measured.

One more thing is new. The paper's exit criterion,
`T < 0.005 · Cost / N_nets`, assumes a walk that started hot; solved for
a target acceptance at an already-good placement, the start temperature
can be *below* that threshold on the first step, and a literal reading
would skip the anneal. The criterion is therefore checked after a step,
and only when that step improved nothing: it describes a walk that has
finished, not one that has not begun. A second exit counts temperatures
that improve nothing while the acceptance rate is in the quench band
(`≤ 0.15`), for a walk that has converged while still nominally warm.

## The acceptance rate, measured

This is what the schedule looks like now on `clock_blink.v` at effort 10.
The window column is the mechanism that was missing.

```
  step   window     tried  accepted  rate   cost
     0        1      4494      2477   55%     454
     4        2      4544      1917   42%     510
     9        1      4498      2202   48%     336
    20        1      4513      1619   35%     317
    40        1      4520      1117   24%     234
    60        1      4504       761   16%     181
    72        1      4484       533   11%     149
```

Three things to read off it:

1. **The walk stays in the same country as where it started.** The worst
   it reaches is 510 against a starting 240, not 5946, and it is back
   under 240 by step 25 and at 144 by the end. The old schedule's best
   step was eight times worse than the placement it was given.
2. **The acceptance rate decays** from 55% through the 0.44 target and
   into the quench band, instead of being pinned at a third by a window
   that cannot shrink. That decay is what lets the cooling table reach
   its `0.8` band and the anneal terminate: 73 temperatures of a 120
   budget, stopped by the temperature criterion.
3. **The window moves, and it is the limiter moving it.** Steps 4 to 11
   widen to 2 tiles because the acceptance rate was above 0.44, and it
   comes back to 1 as soon as the rate does. That is the feedback loop
   working on a design small enough to watch.

The rate cannot fall to zero, and it is worth saying why: an ECP5 logic
tile holds eight lookup tables, so a great many moves change the
tile-granular wirelength by **exactly zero** and are accepted at any
temperature. `R_accept` here has a floor of a few per cent that is a
property of the fabric and not of the walk. It is below the 0.15 band
boundary, so nothing in the schedule is upset by it, but a future cost
function with a sub-tile term would change these numbers.

## What it is worth

### `usb_host_target.v` — 3168 LUT4, 1108 flip-flops, 4304 movable cells

Three numbers per row that are not seconds: the wirelength the annealer
reached, the pips the router then needed, and the nodes its maze
expansions took off the queue. The last is the router's work and means
the same thing on every machine.

| schedule | effort | `place` | `route` | whole flow | wirelength | pips | router nodes |
|---|---|---|---|---|---|---|---|
| before | 10 | 98.1 s | 93.2 s | 193.0 s | 28699 → 23770 | 63 301 | 532 127 182 |
| **after** | **10** | 100.8 s | 90.6 s | 192.3 s | 28699 → **13188** | **55 458** | **494 301 341** |
| after | 3 | 29.3 s | 104.6 s | 134.7 s | 28699 → 13421 | 55 597 | 573 316 176 |
| after | 1 | 10.5 s | 107.1 s | **118.5 s** | 28699 → 14396 | 56 981 | 595 772 795 |

At the default effort the schedule is **free**: the same flow time to a
wirelength 45% shorter and 7 843 fewer pips. The placer does fewer moves
than it used to (44.3 million against 70.7) and each costs more, because
a move now draws from an indexed window and is accepted three times out
of ten instead of being refused; the two cancel.

What the schedule bought is the knob. Effort used to be a lever that only
went the wrong way — the previous round measured effort 3 saving 43 s of
placement and spending **296 s** more on routing, because the router pays
for the placement it is given. It is now a genuine trade: **effort 1 is a
39% faster whole flow than the old default**, and its placement is still
39% shorter than what the old default produced with ten times the moves.

Between runs the router's time varies by about 15% on this machine; the
counted columns do not vary at all. Effort 3's route was 91.0 s on one
run and 104.6 s on another, so read the `route` column of the lower two
rows as "about the same as effort 10's" rather than as a regression.


### `clock_blink.v` — 72 LUT4, 26 flip-flops

The small design, where the old annealer's failure is starkest: it
improved nothing at **any** effort, including 10.

| schedule | effort | `place` | `route` | whole flow | wirelength | pips | router nodes |
|---|---|---|---|---|---|---|---|
| before | 10 | 0.51 s | 0.86 s | 2.29 s | 240 → **240** | 1089 | 5 953 689 |
| **after** | **10** | 0.39 s | 1.23 s | 2.59 s | 240 → **144** | **1014** | 7 434 112 |
| after | 3 | 0.23 s | 1.16 s | 2.36 s | 240 → 164 | 1026 | 7 187 394 |
| after | 1 | 0.18 s | 1.20 s | 2.38 s | 240 → 164 | 1060 | 7 444 816 |

The placement is 40% shorter and needs 75 fewer pips; the flow takes
about the same time either way, because 2.3 seconds of it is a second of
fabric expansion and a second of routing a hundred signals.

**The router does more work on the better placement** — 7.4 million
against 6.0 million nodes off the queue — which is worth stating because
it is the opposite of what happened on the large design. The A\* estimate
charges per tile of Manhattan distance still to cover, so a short
connection is guided less than a long one: a tight placement is a weaker
search problem per net even when it is a better placement. On
`usb_host_target.v` there are four thousand nets and congestion dominates,
so the tighter placement wins there; on a hundred nets with the die
nearly empty, it does not.


## The goldens that moved

Three iCE40 cases in `testdata/fpga/` are placed by this annealer, so
their `.place`, `.route` and `.bits` goldens moved. Every one of them is
accounted for, and one of them is a loss.

| case | wirelength | pips | tiles used | router nodes | set bits |
|---|---|---|---|---|---|
| `blinky_ice40` | 42 → **41** | 198 → **162** | 8 → 10 | 171 700 → **59 674** | 435 → 354 |
| `carry_ice40` | 146 → **115** | 292 → *303* | 32 → **22** | 87 341 → *136 330* | 580 → 607 |
| `ram_ice40` | 186 → 186 | 382 | 19 | 4 539 743 | unchanged |

- `blinky_ice40` is a straight gain: the old annealer improved nothing on
  it either (42 → 42), and the new one finds a placement the router
  serves with 18% fewer pips and a third of the search.
- `carry_ice40` is **a trade and should be read as one.** The placement
  is much better — 146 to 115, into 22 tiles instead of 32 — and the
  router is worse for it: one more rip-up iteration, eleven more pips,
  and 56% more nodes visited. Twenty-two tiles on a synthetic iCE40 with
  eight carry cells is a crowded corner, and this cost function has no
  term that knows that. See "what was not done" below.
- `ram_ice40` does not move at all. Its cells are pinned by the block
  RAM's site and by package pins; there is nothing for a window of one
  tile to do.

`blinky_ice40.asc` moves with its `.bits`, being the same bitstream
written out in full.

Four assertions in `tests/fpga_trellis.rs` moved with the placements, and
three of them should never have been placement-dependent:

| test | was | is |
|---|---|---|
| `a_distributed_ram_and_two_reset_domains_share_a_die` | 57 logic tiles | 65 |
| `a_register_bit_nothing_drives_is_built_from_a_constant` | 199 bits | 180 |
| `the_clocked_design_routes_and_configures_what_its_header_promises` | `> 600` arcs, 693 | 599 |
| `the_bitstream_decodes_back_to_the_arcs_the_router_chose` | `SLICED.B0MUX` by name | any slice input mux |

The first is the one worth reading, because 65 tiles against 57 looks
like a regression and is not. `lutram_reset_64.v` with `fifo_sync`:

| | wirelength | logic tiles | pips | router nodes |
|---|---|---|---|---|
| before | 2668 → **2668** | 57 | 6790 | 46 591 174 |
| after | 2668 → **1289** | 65 | **6396** | **41 997 067** |

The old annealer improved nothing here either, so what it kept was the
legaliser's placement — which packs cells into the fewest tiles it can
and wires them long. The new one spends eight more tiles and halves the
wirelength, and the router needs 394 fewer pips for it. A tile count is
not a quality measure on a part with 24 288 flip-flop sites and 3036
logic tiles for a design using 65 of them.

The two correctness checks are unaffected in kind. `usb_host_target.v`
still decodes every one of its 114 364 set bits back through the database
into exactly the 35 151 arcs the router chose, with none unexplained, and
`every_block_maps_to_the_logic_it_was_mapped_from` does not involve a
placement at all.


## What was not done, and would be next

- **The cost function has no congestion term.** VPR's is
  `Σ q(n) (bb_x/C_av,x + bb_y/C_av,y)`: the `q(n)` factor corrects the
  bounding box for nets with more than three terminals, and the channel
  capacities make a placement pay for crowding a narrow part of the die.
  This placer's cost is plain half-perimeter wirelength in whole tiles.
  `carry_ice40` is what that costs — a placement 31% shorter that the
  router needs an extra iteration and eleven more pips to serve.
- **The move set has no timing term.** Every move is judged by
  wirelength; VPR's timing-driven mode weights a net by its criticality.
  There is a static timing engine in this crate (`timing::sta`) and
  nothing connects it to the placer.
- **A move is still proposed before it is judged legal.** On the ECP5 the
  shared-pin check (`SiteRules`) rejects a fraction of the window's
  offers, and a generator that knew which sites of the window were
  compatible would not have to.
