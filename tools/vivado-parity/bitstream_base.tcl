# Implements options_probe once and keeps the checkpoint, so that every option
# is written from one routing.
#   vivado -mode batch -source bitstream_base.tcl -tclargs <fixture dir> <out>
lassign $argv dir out
file mkdir $out
cd $out
read_verilog $dir/options_probe.v
read_xdc $dir/options_probe.xdc
synth_design -top options_probe -part xc7a100tftg256-1
opt_design
place_design
route_design
write_checkpoint -force options_probe.dcp
write_bitstream -force -bin_file -raw_bitfile -mask_file -logic_location_file -readback_file base.bit
set f [open properties.txt w]
foreach p [lsort [list_property [current_design] BITSTREAM.*]] {
  puts $f "$p [list_property_value $p [current_design]]"
}
foreach p {CONFIG_MODE CFGBVS CONFIG_VOLTAGE} {
  puts $f "$p [list_property_value $p [current_design]]"
}
close $f
