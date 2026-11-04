# What Vivado did with one design of testdata/fpga/vivado/muxf/, in its own
# words: where each wide multiplexer went and what is on its pins, and the
# timing of the worst path, element by element.
#   vivado -mode batch -source muxf_probe.tcl -tclargs <dir> <top> <part> <out>
lassign $argv dir top part out
file mkdir $out
cd $out
read_verilog $dir/$top.v
read_xdc $dir/$top.xdc
synth_design -top $top -part $part -quiet
opt_design -quiet
place_design -quiet
route_design -quiet
set f [open $top.cells w]
proc pin_end {p} {
  # The cell and BEL at the far end of what drives or reads this pin.
  set out {}
  foreach net [get_nets -quiet -of_objects $p] {
    foreach o [get_pins -quiet -of_objects $net -filter {DIRECTION == OUT && IS_LEAF}] {
      set c [get_cells -of_objects $o]
      lappend out "[get_property REF_NAME $c]/[get_property REF_PIN_NAME $o]@[get_property BEL $c]@[get_property LOC $c]"
    }
    foreach p [get_ports -quiet -of_objects $net] { lappend out "port:$p" }
  }
  return $out
}
proc pin_reads {p} {
  set out {}
  foreach net [get_nets -quiet -of_objects $p] {
    foreach o [get_pins -quiet -of_objects $net -filter {DIRECTION == IN && IS_LEAF}] {
      set c [get_cells -of_objects $o]
      lappend out "[get_property REF_NAME $c]/[get_property REF_PIN_NAME $o]@[get_property BEL $c]@[get_property LOC $c]"
    }
  }
  return $out
}
foreach ref [list MUXF7 MUXF8] {
  foreach c [lsort [get_cells -quiet -hier -filter "REF_NAME == $ref"]] {
    puts $f "cell $ref bel [get_property BEL $c] loc [get_property LOC $c]"
    foreach pin [list I0 I1 S] {
      puts $f "  $pin from [pin_end [get_pins $c/$pin]]"
    }
    puts $f "  O to [lsort [pin_reads [get_pins $c/O]]]"
  }
}
set counts [dict create]
foreach c [get_cells -hier -filter {IS_PRIMITIVE}] { dict incr counts [get_property REF_NAME $c] }
puts $f "counts [lsort -stride 2 $counts]"
close $f
report_timing -delay_type max -max_paths 1 -path_type full -input_pins -file $top.timing -quiet
write_bitstream -force -quiet $top.bit
