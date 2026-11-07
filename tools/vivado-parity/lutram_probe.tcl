# What Vivado did with one design of testdata/fpga/vivado/lutram/ or bram/, in its
# own words: every memory cell it left, with its BEL and what reaches each pin,
# and a bitstream to decode.
#   vivado -mode batch -source lutram_probe.tcl -tclargs <dir> <top> <part> <out>
lassign $argv dir top part out
file mkdir $out
cd $out
read_verilog $dir/$top.v
read_xdc $dir/$top.xdc
synth_design -top $top -part $part -quiet
set f [open $top.cells w]
set counts [dict create]
foreach c [get_cells -hier -filter {IS_PRIMITIVE}] { dict incr counts [get_property REF_NAME $c] }
puts $f "synth [lsort -stride 2 $counts]"
opt_design -quiet
place_design -quiet
route_design -quiet
set counts [dict create]
foreach c [get_cells -hier -filter {IS_PRIMITIVE}] { dict incr counts [get_property REF_NAME $c] }
puts $f "routed [lsort -stride 2 $counts]"
# Each block RAM's arrangement, as its parameters say it.
foreach c [lsort [get_cells -hier -filter {IS_PRIMITIVE && REF_NAME =~ RAMB*}]] {
  set words {}
  foreach p {RAM_MODE READ_WIDTH_A READ_WIDTH_B WRITE_WIDTH_A WRITE_WIDTH_B DOA_REG DOB_REG
             WRITE_MODE_A WRITE_MODE_B RAM_EXTENSION_A RAM_EXTENSION_B} {
    if {[llength [list_property $c $p]]} { lappend words "$p=[get_property $p $c]" }
  }
  puts $f "params $c [get_property REF_NAME $c] [join $words " "]"
}
foreach c [lsort [get_cells -hier -filter {IS_PRIMITIVE && (REF_NAME =~ RAM* || REF_NAME =~ FIFO*)}]] {
  set parent [get_cells -quiet [get_property PARENT $c]]
  set pref ""
  if {[llength $parent]} { set pref [get_property REF_NAME $parent] }
  puts $f "cell $c [get_property REF_NAME $c] parent $pref bel [get_property BEL $c] site [get_property LOC $c]"
  foreach p [lsort [get_pins -of_objects $c]] {
    set net [get_nets -quiet -of_objects $p]
    set bel ""
    foreach b [get_bel_pins -quiet -of_objects $p] { lappend bel [lindex [split $b /] end] }
    puts $f "  pin [get_property REF_PIN_NAME $p] net $net bel $bel"
  }
}
# Which site pins each memory cell's nets use at its own site, and the site's
# muxes as Vivado set them.
set sites {}
foreach c [get_cells -hier -filter {IS_PRIMITIVE && (REF_NAME =~ RAM* || REF_NAME =~ FIFO*)}] {
  set s [get_property LOC $c]
  if {$s eq ""} { continue }
  lappend sites $s
  foreach p [lsort [get_pins -of_objects $c]] {
    set net [get_nets -quiet -of_objects $p]
    if {$net eq ""} { continue }
    set here {}
    foreach sp [get_site_pins -quiet -of_objects $net] {
      if {[string match "$s/*" $sp]} { lappend here [lindex [split $sp /] end] }
    }
    puts $f "sitepin $c [get_property REF_PIN_NAME $p] [lsort $here]"
  }
}
foreach s [lsort -unique $sites] {
  puts $f "sitepips $s [lsort [get_property SITE_PIPS [get_sites $s]]]"
}
# A FIFO's site in full: every site pin and the net on it, a pin no net
# reaches written `-`, so that what Vivado leaves alone is an answer too.
foreach c [lsort [get_cells -hier -filter {IS_PRIMITIVE && REF_NAME =~ FIFO*}]] {
  set s [get_property LOC $c]
  if {$s eq ""} { continue }
  foreach sp [lsort [get_site_pins -of_objects [get_sites $s]]] {
    set net [get_nets -quiet -of_objects $sp]
    if {$net eq ""} { set net "-" }
    puts $f "allsitepin $c [lindex [split $sp /] end] $net"
  }
}
close $f
# Vivado's own netlist of what it built, on its simulation models, for
# gen_lutram_vectors.py to ask xsim about.
write_verilog -force -mode funcsim $top.funcsim.v
report_utilization -file $top.util -quiet
report_timing -delay_type max -max_paths 1 -path_type full -input_pins -file $top.timing -quiet
set_property SEVERITY Warning [get_drc_checks NSTD-1]
set_property SEVERITY Warning [get_drc_checks UCIO-1]
# A refusal is an answer too, and `-quiet` would hide it.
if {[catch {write_bitstream -force $top.bit} message]} {
  set f [open $top.cells a]
  puts $f "bitstream_refused [string map {"\n" " "} $message]"
  foreach v [get_drc_violations -quiet -filter {SEVERITY == Error}] { puts $f "drc [get_property NAME $v] [get_property DESCRIPTION $v]" }
  close $f
}
