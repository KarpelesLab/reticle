clock domain crossings in module `cdc_reconverge`: 2 domain(s), 3 crossing(s)
  domain           clock net        cells
  aclk             aclk                 2
  bclk             bclk                 5

warning: gray bus aclk -> bclk (unverified)
  2 bits cross together through separate synchronisers; no gray-code generator was found in the source cone. Synchronising a multi-bit bus is only safe if at most one bit changes per source clock, which this analysis cannot prove
  source: srcff0, srcff1
  dest:   sync0a, sync1a

note: synchroniser aclk -> bclk
  `sync0a` starts a 2-flop synchroniser with no logic in the chain
  source: srcff0
  dest:   sync0a

note: synchroniser aclk -> bclk
  `sync1a` starts a 2-flop synchroniser with no logic in the chain
  source: srcff1
  dest:   sync1a

warning: reconvergence aclk -> bclk
  2 synchronised bits (sync0b, sync1b) are recombined at `recombine/y`
