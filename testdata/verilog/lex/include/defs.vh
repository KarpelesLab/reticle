`ifndef DEFS_VH
`define DEFS_VH
`define DATA_W 16
`define DECL_PORT(name) input wire [`DATA_W-1:0] name
`include "nested/inner.vh"
`endif
