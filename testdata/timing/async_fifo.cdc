clock domain crossings in module `async_fifo`: 2 domain(s), 6 crossing(s)
  domain           clock net        cells
  rclk             rclk                 5
  wclk             wclk                 5

warning: gray bus rclk -> wclk (unverified)
  2 bits cross together through separate synchronisers; the source cone does contain an XOR of a value with a shifted copy of itself, the usual gray-code generator. Synchronising a multi-bit bus is only safe if at most one bit changes per source clock, which this analysis cannot prove
  source: rgray_reg
  dest:   rg_sync1

warning: async fifo wclk -> rclk (unverified)
  memory `ram` is written in one domain and read in another, and synchronised buses cross in both directions, which is what an asynchronous FIFO with gray pointers looks like; that the pointers are really gray-coded cannot be proved here
  source: wr
  dest:   rd

warning: gray bus wclk -> rclk (unverified)
  2 bits cross together through separate synchronisers; the source cone does contain an XOR of a value with a shifted copy of itself, the usual gray-code generator. Synchronising a multi-bit bus is only safe if at most one bit changes per source clock, which this analysis cannot prove
  source: wgray_reg
  dest:   wg_sync1

note: synchroniser rclk -> wclk
  `rg_sync1` starts a 2-flop synchroniser with no logic in the chain
  source: rgray_reg
  dest:   rg_sync1

note: handshake wclk -> rclk (unverified)
  synchronised signals cross in both directions, which is the shape of a request / acknowledge handshake; whether the two are actually a protocol cannot be told from the netlist

note: synchroniser wclk -> rclk
  `wg_sync1` starts a 2-flop synchroniser with no logic in the chain
  source: wgray_reg
  dest:   wg_sync1
