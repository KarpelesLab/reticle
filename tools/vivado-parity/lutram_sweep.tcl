# Which memories Vivado's synthesis takes to block RAM and which to distributed:
# one line per design of the list given, its memory, flip-flop and table cells
# after synth_design.
#   vivado -mode batch -source lutram_sweep.tcl -tclargs <dir> <list> <part> <out>
lassign $argv dir list part out
set o [open $out w]
set l [open $list]
foreach top [split [string trim [read $l]] "\n"] {
  read_verilog $dir/$top.v
  if {[catch {synth_design -top $top -part $part -quiet} m]} {
    puts $o "$top failed [string map {"\n" " "} $m]"
    flush $o
    catch { close_design }
    remove_files -quiet [get_files -quiet]
    continue
  }
  set counts [dict create]
  foreach c [get_cells -hier -filter {IS_PRIMITIVE}] {
    set r [get_property REF_NAME $c]
    if {[string match RAM* $r] || [string match FIFO* $r] || [string match FD* $r]} { dict incr counts $r }
  }
  puts $o "$top [lsort -stride 2 $counts]"
  flush $o
  close_design
  remove_files -quiet [get_files -quiet]
}
close $o
