# What Vivado did with one design of testdata/fpga/vivado/config/, in its own
# words: where each configuration primitive went, the properties it ended with,
# what each pin is on, and the route of every net reaching one. A step that fails
# says which step it was, and the message: a refusal is an answer here.
#   vivado -mode batch -source config_probe.tcl -tclargs <dir> <top> <part> <out>
lassign $argv dir top part out
file mkdir $out
cd $out
set f [open $top.cells w]
proc say {args} { global f; puts $f [join $args " "]; flush $f }
proc step {name script} {
  if {[catch {uplevel 1 $script} message]} {
    say failed $name [string map {"\n" " "} $message]
    say end
    close $::f
    exit 0
  }
}
step read { read_verilog $dir/$top.v; read_xdc $dir/$top.xdc }
step synth { synth_design -top $top -part $part -quiet }
step opt { opt_design -quiet }
step place { place_design }
step route { route_design }
set kinds {BSCANE2 CAPTUREE2 DNA_PORT EFUSE_USR FRAME_ECCE2 ICAPE2 STARTUPE2 USR_ACCESSE2}
foreach c [lsort [get_cells -hier -filter {IS_PRIMITIVE}]] {
  set ref [get_property REF_NAME $c]
  if {[lsearch $kinds $ref] < 0} { continue }
  say cell $c ref $ref site [get_property LOC $c] bel [get_property BEL $c]
  foreach p [lsort [list_property $c]] {
    if {[regexp {^(JTAG_CHAIN|DISABLE_JTAG|ONESHOT|SIM_.*|FARSRC|FRAME_RBT_IN_FILENAME|ICAP_WIDTH|DEVICE_ID|PROG_USR)$} $p]} {
      say "  prop" $p [get_property $p $c]
    }
  }
  foreach pin [lsort [get_pins -of_objects $c]] {
    set net [get_nets -quiet -of_objects $pin]
    set type {}
    set route {}
    if {[llength $net]} {
      set type [get_property TYPE $net]
      catch { set route [get_property ROUTE $net] }
    }
    say "  pin" [get_property REF_PIN_NAME $pin] [get_property DIRECTION $pin] $type $route
  }
}
foreach p [lsort [list_property [current_design] BITSTREAM.*]] {
  set v [get_property $p [current_design]]
  if {$v ne ""} { say option $p $v }
}
step bitstream {
  write_bitstream -force $top.bit
  if {![file exists $top.bit]} { error "write_bitstream wrote no file" }
}
say end
close $f
