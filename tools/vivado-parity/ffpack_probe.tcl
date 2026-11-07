# What Vivado did with one design of testdata/fpga/vivado/ffpack/: the cells of
# each slice the held flip-flops (`r*`) are in, the site pips it used there, and
# the route of each net that reaches one of those flip-flops; their data paths'
# timing; then a bitstream.
#   vivado -mode batch -source ffpack_probe.tcl -tclargs <dir> <top> <part> <out>
# A design Vivado refuses leaves <out>/<top>.cells with `refused` and the step.
lassign $argv dir top part out
file mkdir $out
cd $out
set f [open $top.cells w]
read_verilog $dir/$top.v
read_xdc $dir/$top.xdc
foreach step {synth opt place route} {
  switch $step {
    synth { set cmd {synth_design -top $top -part $part -quiet} }
    opt { set cmd {opt_design -quiet} }
    place { set cmd {place_design -quiet} }
    route { set cmd {route_design -quiet} }
  }
  if {[catch $cmd message]} {
    puts $f "refused $step [string map {"\n" " "} $message]"
    close $f
    exit 0
  }
}
set held [lsort -dictionary [get_cells -regexp {r[0-9]+}]]
set sites {}
foreach c $held {
  set s [get_sites -of_objects $c]
  if {[lsearch $sites $s] < 0} { lappend sites $s }
}
foreach site $sites {
  puts $f "site $site [get_property SITE_TYPE $site] [get_tiles -of_objects $site]"
  foreach c [lsort [get_cells -of_objects $site]] {
    puts $f "  cell [get_property BEL $c] $c [get_property REF_NAME $c]"
  }
  foreach p [lsort [get_site_pips -quiet -of_objects $site -filter {IS_USED}]] {
    puts $f "  sitepip $p"
  }
  foreach b [lsort [get_bels -quiet -of_objects $site -filter {IS_USED}]] {
    puts $f "  bel $b"
  }
}
foreach c $held {
  set n [get_nets -of_objects [get_pins $c/D]]
  puts $f "  net $c/D $n route [get_property ROUTE $n]"
}
set unrouted [get_nets -quiet -hier -filter {ROUTE_STATUS != INTRASITE && ROUTE_STATUS != ROUTED && ROUTE_STATUS != NOLOADS && ROUTE_STATUS != NODRIVER}]
puts $f "  unrouted [llength $unrouted] $unrouted"
close $f
# Each held flip-flop's data path, the route through a table and the bypass
# input side by side.
set t [open $top.timing w]
close $t
foreach c $held {
  report_timing -to [get_pins $c/D] -delay_type max -path_type full -input_pins -append -file $top.timing -quiet
}
write_bitstream -force -quiet $top.bit
