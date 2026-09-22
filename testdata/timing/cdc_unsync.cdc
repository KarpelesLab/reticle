clock domain crossings in module `cdc_unsync`: 2 domain(s), 1 crossing(s)
  domain           clock net        cells
  aclk             aclk                 1
  bclk             bclk                 1

error: unsynchronised aclk -> bclk
  `dst` captures data from another clock domain and combinational logic sits between the domains
  source: src
  dest:   dst
