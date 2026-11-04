# One design through Vivado, with what became of its DSP48E1s written down.
#   vivado -mode batch -source dsp_facts.tcl -tclargs <dir> <top> <part> <out>
# <out>/<top>.facts has each block's site, parameters and what each pin is on,
# then every other primitive; <top>.ww.rpt is the worst register path and
# <top>.bit the bitstream, for `reticle fpga --decode`.
lassign $argv dir top part out
file mkdir $out
cd $out
read_verilog $dir/$top.v
read_xdc $dir/$top.xdc
synth_design -top $top -part $part
opt_design
place_design
route_design
write_bitstream -force $top.bit
set f [open $top.facts w]
foreach c [lsort [get_cells -hier -filter {REF_NAME == DSP48E1}]] {
  puts $f "cell $c loc [get_property LOC $c] bel [get_property BEL $c]"
  foreach p {ACASCREG ADREG ALUMODEREG AREG BCASCREG BREG CARRYINREG CARRYINSELREG CREG DREG INMODEREG MREG OPMODEREG PREG A_INPUT B_INPUT USE_MULT USE_DPORT USE_SIMD USE_PATTERN_DETECT AUTORESET_PATDET MASK PATTERN SEL_MASK SEL_PATTERN IS_CLK_INVERTED IS_CARRYIN_INVERTED IS_ALUMODE_INVERTED IS_INMODE_INVERTED IS_OPMODE_INVERTED} {
    puts $f "  param $p [get_property $p $c]"
  }
  foreach pin [lsort [get_pins -of_objects $c]] {
    set n [get_nets -quiet -of_objects $pin]
    set t ""
    if {[llength $n]} { set t [get_property TYPE $n] }
    puts $f "  pin [get_property REF_PIN_NAME $pin] [get_property DIRECTION $pin] net=$n type=$t"
  }
}
foreach c [lsort [get_cells -hier -filter {IS_PRIMITIVE && REF_NAME != DSP48E1}]] {
  puts $f "other [get_property REF_NAME $c] $c [get_property LOC $c] [get_property BEL $c]"
}
close $f
report_timing -from [all_registers -clock_pins] -to [all_registers -data_pins] -max_paths 1 -file $top.ww.rpt
report_timing -through [get_pins -hier -filter {REF_PIN_NAME =~ P[*]}] -max_paths 1 -file $top.tp.rpt
