# A design from gen_ecc_design.py, routed and written three ways: as it stands,
# compressed and with a CRC per frame.
#   vivado -mode batch -source bitstream_ecc.tcl -tclargs <v> <top> <part> <out> <clk> <si> <so>
lassign $argv v top part out clk si so
file mkdir $out; cd $out
read_verilog $v
synth_design -top $top -part $part -flatten_hierarchy none
create_clock -period 20 -name clk [get_ports clk]
set_property PACKAGE_PIN $clk [get_ports clk]
set_property PACKAGE_PIN $si [get_ports si]
set_property PACKAGE_PIN $so [get_ports so]
set_property IOSTANDARD LVCMOS33 [get_ports *]
opt_design
place_design
route_design
write_checkpoint -force $top.dcp
write_bitstream -force $top.bit
set_property BITSTREAM.GENERAL.COMPRESS TRUE [current_design]
write_bitstream -force ${top}_compress.bit
reset_property BITSTREAM.GENERAL.COMPRESS [current_design]
set_property BITSTREAM.GENERAL.PERFRAMECRC YES [current_design]
write_bitstream -force ${top}_perframe.bit
