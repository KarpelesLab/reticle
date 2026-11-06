# What a synthesised netlist is made of and how deep its logic is, by Vivado's
# own counting, written to <out>/<top>.qor:
#   vivado -mode batch -source netlist_qor.tcl -tclargs <stage> <stem> <top> <part> <out>
#   synth:   <stem>.v and <stem>.xdc through Vivado's synth_design, nothing after.
#   netlist: Reticle's netlist (`reticle fpga --vivado-placement <stem>.tcl`), linked
#            as it is, unplaced.
# Lines: `cells <group> <n>` for LUT (each LUTn too), CARRY4, FF, MUXF7, MUXF8, SRL,
# LUTRAM, BRAM and DSP; `levels <clock> max <n> worst <n> luts <n> carries <n>`, the
# most levels of logic on any of the clock's paths to itself in
# report_design_analysis's distribution (its 5000 worst paths), and the levels of
# the worst setup path with how many of them are tables and how many CARRY4s.
lassign $argv stage stem top part out
file mkdir $out
cd $out
set result [open $top.qor w]
proc say {args} { global result; puts $result [join $args " "]; flush $result }
say stage $stage part $part vivado [version -short]
if {$stage eq "synth"} {
  read_verilog $stem.v
  read_xdc $stem.xdc
  synth_design -top $top -part $part
} else {
  set_property design_mode GateLvl [current_fileset]
  read_verilog $stem.v
  read_xdc $stem.xdc
  link_design -top $top -part $part
}
set leaves [get_cells -quiet -hier -filter {IS_PRIMITIVE}]
proc count {filter} {
  global leaves
  return [llength [filter -quiet $leaves $filter]]
}
say cells LUT [count {PRIMITIVE_GROUP == LUT}]
foreach n {1 2 3 4 5 6} { say cells LUT$n [count "REF_NAME == LUT$n"] }
say cells CARRY4 [count {REF_NAME == CARRY4}]
say cells FF [count {PRIMITIVE_GROUP == FLOP_LATCH}]
say cells MUXF7 [count {REF_NAME == MUXF7}]
say cells MUXF8 [count {REF_NAME == MUXF8}]
say cells SRL [count {PRIMITIVE_SUBGROUP == SRL}]
say cells LUTRAM [count {PRIMITIVE_GROUP == DMEM}]
say cells BRAM [count {PRIMITIVE_GROUP == BLOCKRAM}]
say cells DSP [count {PRIMITIVE_GROUP == MULT}]
report_design_analysis -logic_level_distribution -logic_level_dist_paths 5000 \
  -file [file join [pwd] levels.rpt]
foreach clock [get_clocks -quiet] {
  set paths [get_timing_paths -quiet -from $clock -to $clock -max_paths 5000 -nworst 1 -setup]
  set max 0
  foreach p $paths {
    set l [get_property LOGIC_LEVELS $p]
    if {$l > $max} { set max $l }
  }
  set worst [lindex $paths 0]
  if {$worst eq ""} { continue }
  set cells [get_cells -quiet -of_objects $worst]
  set luts [llength [filter -quiet $cells {PRIMITIVE_GROUP == LUT}]]
  set carries [llength [filter -quiet $cells {REF_NAME == CARRY4}]]
  say levels $clock max $max worst [get_property LOGIC_LEVELS $worst] luts $luts carries $carries
}
say end
