# Each ground and power pin of a routed checkpoint's sites, and the pips of its
# net back to the source: `vivado -mode batch -source ground_paths.tcl -tclargs <dcp> <out>`.
lassign $argv dcp out
open_checkpoint $dcp
set f [open $out w]
foreach net [get_nets -hierarchical -filter {TYPE == GROUND || TYPE == POWER}] {
  set pips [get_pips -quiet -of $net]
  set into [dict create]
  foreach p $pips {
    set dn [get_nodes -quiet -of $p -downhill]
    dict set into $dn $p
  }
  foreach pin [get_pins -of $net -filter {DIRECTION == IN}] {
    set sp [get_site_pins -quiet -of $pin]
    if {$sp eq ""} continue
    set node [get_nodes -quiet -of $sp]
    set chain {}
    for {set i 0} {$i < 12} {incr i} {
      if {![dict exists $into $node]} break
      set p [dict get $into $node]
      lappend chain $p
      set node [get_nodes -quiet -of $p -uphill]
    }
    puts $f "[get_property TYPE $net] $sp [join $chain { < }]"
  }
}
close $f
