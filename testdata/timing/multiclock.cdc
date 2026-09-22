clock domain crossings in module `multiclock`: 2 domain(s), 1 crossing(s)
  domain           clock net        cells
  fast             fast                 1
  slow             slow                 1

error: unsynchronised fast -> slow
  `dst` captures data from another clock domain and combinational logic sits between the domains
  source: src
  dest:   dst
