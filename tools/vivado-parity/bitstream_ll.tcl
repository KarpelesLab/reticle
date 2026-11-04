# Writes the logic location file of ll_probe, with its flip-flops on every bel
# of four slices, and quotes the lines for it.
#   vivado -mode batch -source bitstream_ll.tcl -tclargs <fixture dir> <out>
lassign $argv dir out
file mkdir $out
cd $out
read_verilog $dir/ll_probe.v
synth_design -top ll_probe -part xc7a100tftg256-1
create_clock -period 10 -name clk [get_ports clk]
set_property PACKAGE_PIN N11 [get_ports clk]
set_property PACKAGE_PIN L4 [get_ports si]
set_property PACKAGE_PIN L5 [get_ports so]
set_property IOSTANDARD LVCMOS33 [get_ports *]
set i 0
foreach slice {SLICE_X0Y0 SLICE_X1Y0 SLICE_X2Y1 SLICE_X3Y1} {
  foreach bel {AFF A5FF BFF B5FF CFF C5FF DFF D5FF} {
    set cell [get_cells g[$i].f]
    set_property BEL $bel $cell
    set_property LOC $slice $cell
    incr i
  }
}
set_property BEL RAMB18E1 [get_cells r18]
set_property LOC RAMB18_X0Y1 [get_cells r18]
set_property BEL RAMB36E1 [get_cells r36]
set_property LOC RAMB36_X1Y1 [get_cells r36]
opt_design
place_design
route_design
write_bitstream -force -logic_location_file ll_probe.bit
