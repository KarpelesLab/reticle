# What Vivado's design rule check says of every pair of (standard, direction) in
# one bank, slice i of n, with no placement: the check comes before the placer.
#   vivado -mode batch -source io_pairs.tcl -tclargs <i> <n> <out>
# io_pairs.sh runs the slices, and io_pairs.py makes testdata/fpga/vivado/io/pairs.txt.
lassign $argv slice nslices out
set here [file dirname [info script]]
read_verilog $here/io_pairs.v
synth_design -top pairs -part xc7a100tftg256-1 -quiet
set SE {LVTTL LVCMOS33 LVCMOS25 LVCMOS18 LVCMOS15 LVCMOS12 HSUL_12 HSTL_I HSTL_II HSTL_I_18 HSTL_II_18 SSTL18_I SSTL18_II SSTL15 SSTL15_R SSTL135 SSTL135_R PCI33_3 MOBILE_DDR}
set DF {DIFF_HSTL_I DIFF_HSTL_II DIFF_HSTL_I_18 DIFF_HSTL_II_18 DIFF_SSTL18_I DIFF_SSTL18_II DIFF_SSTL15 DIFF_SSTL15_R DIFF_SSTL135 DIFF_SSTL135_R DIFF_HSUL_12 DIFF_MOBILE_DDR BLVDS_25 LVDS_25 RSDS_25 TMDS_33 MINI_LVDS_25 PPDS_25}
set NOBI {HSTL_I HSTL_I_18 SSTL18_I DIFF_HSTL_I DIFF_HSTL_I_18 DIFF_SSTL18_I PPDS_25 RSDS_25 TMDS_33}
# the items: {standard kind}
set items {}
foreach s $SE {
  lappend items [list $s in] [list $s out]
  if {[lsearch $NOBI $s] < 0} { lappend items [list $s io] }
}
foreach s $DF {
  lappend items [list $s din] [list $s dout]
  if {[lsearch $NOBI $s] < 0} { lappend items [list $s dio] }
}
lappend items {LVDS_25 dinT} {RSDS_25 dinT} {MINI_LVDS_25 dinT} {PPDS_25 dinT}
# parking: pads in bank 14 and 34 at LVCMOS33
proc ports_of {k n} {
  switch -glob $k {
    d* { return [list ${k}${n}p ${k}${n}n] }
    default { return [list ${k}${n}] }
  }
}
array set park {in1 N11 in2 N12 out1 N13 out2 N14 io1 P10 io2 P11 din1p A8 din1n A9 din2p B9 din2n A10 dout1p B10 dout1n B11 dout2p B12 dout2n A12 dio1p A13 dio1n A14 dio2p B15 dio2n A15 dn K12 en P6 r T7}
proc parkstd {p} { if {[string match d?* $p] && $p ne "dn"} { return LVDS_25 } ; return LVCMOS33 }
foreach p [array names park] { set_property IOSTANDARD [parkstd $p] [get_ports $p]; set_property PACKAGE_PIN $park($p) [get_ports $p] }
set f [open $out w]
proc rk {k} { if {$k eq "dinT"} { return din } ; return $k }
set idx 0
foreach a $items {
  foreach b $items {
    incr idx
    if {($idx % $nslices) != $slice} continue
    lassign $a s1 k1; lassign $b s2 k2
    set r1 [rk $k1]; set r2 [rk $k2]
    set p1 [ports_of $r1 1]; set p2 [ports_of $r2 2]
    set bank35 {{G2 G1} {H2 H1}}
    # move to bank 35
    set j 0
    foreach p $p1 ball [lindex $bank35 0] { if {$p ne ""} { set_property IOSTANDARD $s1 [get_ports $p]; set_property PACKAGE_PIN $ball [get_ports $p] } }
    foreach p $p2 ball [lindex $bank35 1] { if {$p ne ""} { set_property IOSTANDARD $s2 [get_ports $p]; set_property PACKAGE_PIN $ball [get_ports $p] } }
    if {$k1 eq "dinT"} { foreach p $p1 { set_property DIFF_TERM TRUE [get_ports $p] } }
    if {$k2 eq "dinT"} { foreach p $p2 { set_property DIFF_TERM TRUE [get_ports $p] } }
    report_drc -quiet -name d -checks {BIVC-1 BIVR-1 BIVRU-1 BIVB-1 IOSTDTYPE-1} -file /dev/null
    set vs {}
    foreach v [get_drc_violations -quiet -name d] { lappend vs [get_property CHECK $v] }
    set vs [lsort -unique $vs]
    set msg ""
    foreach v [get_drc_violations -quiet -name d] { if {[get_property CHECK $v] in {BIVC-1 BIVR-1}} { set msg [string map {"\n" " "} [get_property DESCRIPTION $v]]; break } }
    puts $f "$s1 $k1 $s2 $k2 [expr {[llength $vs] ? [join $vs ,] : {ok}}] $msg"
    # park again
    foreach p [concat $p1 $p2] { if {$p ne ""} { set_property DIFF_TERM FALSE [get_ports $p]; set_property IOSTANDARD [parkstd $p] [get_ports $p]; set_property PACKAGE_PIN $park($p) [get_ports $p] } }
  }
  flush $f
}
close $f
