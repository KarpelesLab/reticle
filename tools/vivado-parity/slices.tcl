# What each slice holds, in Vivado's words, for one design: Vivado's own flow on
# it, or Reticle's placement of it loaded unrouted.
#   vivado -mode batch -source slices.tcl -tclargs <stage> <stem> <top> <part> <out>
#   own:    <stem>.v and <stem>.xdc through synth_design, opt_design,
#           place_design, phys_opt_design and route_design, as achieved.tcl's
#           `own` stage.
#   placed: Reticle's netlist (`reticle fpga --vivado-placement <stem>.tcl`)
#           linked and its placement sourced, nothing routed.
# Into <out>: sites.txt, each site with a cell, `site <site> <type> <tile>` and
# under it `  cell <bel> <cell> <primitive>`; nets.txt, each signal with one
# driver, `net <name> <loads> <driver pin> <load pins>`; and util.rpt.
# slices.py reads them.
lassign $argv stage stem top part out
file mkdir $out
cd $out
if {$stage eq "own"} {
  read_verilog $stem.v
  read_xdc $stem.xdc
  synth_design -top $top -part $part
  opt_design
  place_design
  phys_opt_design
  route_design
} else {
  set_property design_mode GateLvl [current_fileset]
  read_verilog $stem.v
  read_xdc $stem.xdc
  link_design -top $top -part $part
  source $stem.tcl
}
set f [open $out/sites.txt w]
foreach s [lsort [get_sites -quiet -of_objects [get_cells -quiet -hier -filter {IS_PRIMITIVE}]]] {
  puts $f "site $s [get_property SITE_TYPE $s] [get_tiles -of_objects $s]"
  foreach c [lsort [get_cells -quiet -of_objects $s]] {
    puts $f "  cell [get_property BEL $c] $c [get_property REF_NAME $c]"
  }
}
close $f
set f [open $out/nets.txt w]
foreach n [get_nets -quiet -hier -filter {TYPE == SIGNAL}] {
  set d [get_pins -quiet -leaf -of_objects $n -filter {DIRECTION == OUT}]
  set l [get_pins -quiet -leaf -of_objects $n -filter {DIRECTION == IN}]
  if {[llength $d] != 1} continue
  puts $f "net $n [llength $l] $d $l"
}
close $f
report_utilization -file $out/util.rpt
puts "end"
