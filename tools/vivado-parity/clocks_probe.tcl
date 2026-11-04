# What Vivado did with one design of testdata/fpga/vivado/clocks/, in its own
# words: where each clock buffer and clock manager went, the properties it
# ended with, and what each pin is on. A step that fails says which step it was,
# and the message: a refusal is an answer here.
#   vivado -mode batch -source clocks_probe.tcl -tclargs <dir> <top> <part> <out>
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
set kinds {BUFG* BUFH* BUFR BUFIO BUFMR* MMCM* PLL* IBUF* DIFF*}
foreach c [lsort [get_cells -hier -filter {IS_PRIMITIVE}]] {
  set ref [get_property REF_NAME $c]
  set hit 0
  foreach k $kinds { if {[string match $k $ref]} { set hit 1 } }
  if {!$hit} { continue }
  say cell $c ref $ref orig [get_property ORIG_REF_NAME $c] site [get_property LOC $c] bel [get_property BEL $c]
  foreach p [lsort [list_property $c]] {
    if {[regexp {^(BUFR_DIVIDE|CE_TYPE|INIT_OUT|PRESELECT_I.|IS_.*_INVERTED|COMPENSATION|BANDWIDTH|CLK.*_(DIVIDE|DIVIDE_F|MULT|MULT_F|PHASE|DUTY_CYCLE|USE_FINE_PS|CASCADE|PERIOD)|DIVCLK_DIVIDE|STARTUP_WAIT|SS_.*|REF_JITTER.)$} $p]} {
      say "  prop" $p [get_property $p $c]
    }
  }
  foreach pin [lsort [get_pins -of_objects $c]] {
    set net [get_nets -quiet -of_objects $pin]
    set type {}
    if {[llength $net]} { set type [get_property TYPE $net] }
    say "  pin" [get_property REF_PIN_NAME $pin] [get_property DIRECTION $pin] $type
  }
}
set counts [dict create]
foreach c [get_cells -hier -filter {IS_PRIMITIVE}] { dict incr counts [get_property REF_NAME $c] }
say counts [lsort -stride 2 $counts]
foreach n [lsort [get_nets -hier -filter {TYPE == GLOBAL_CLOCK || TYPE == REGIONAL_CLOCK || TYPE == LOCAL_CLOCK}]] {
  catch { say route $n [get_property TYPE $n] [get_property ROUTE $n] }
}
write_checkpoint -force $top.dcp
step bitstream {
  write_bitstream -force $top.bit
  if {![file exists $top.bit]} { error "write_bitstream wrote no file" }
}
say end
close $f
