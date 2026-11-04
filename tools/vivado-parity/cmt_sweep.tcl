# One routed clock manager, its parameters changed and a bitstream written for
# each line of a list, so that what a parameter does to the registers can be read
# without placing and routing it each time.
#   vivado -mode batch -source cmt_sweep.tcl -tclargs <dcp> <list> <out>
# Each line of <list> is `<tag> <property> <value> ...`, and this writes
# <out>/<tag>.bit or a line saying why not in <out>/<tag>.refused.
lassign $argv dcp list out
file mkdir $out
open_checkpoint $dcp
set cell [get_cells dut]
# Vivado's rules on what a manager may be set to stay on: a refusal is an answer.
set_property SEVERITY {Warning} [get_drc_checks NSTD-1]
set_property SEVERITY {Warning} [get_drc_checks UCIO-1]
set f [open $list r]
foreach line [split [read $f] "\n"] {
  if {[llength $line] == 0} { continue }
  set tag [lindex $line 0]
  # A run killed by the machine's memory can be started again: what it did stands.
  if {[file exists $out/$tag.bit] || [file exists $out/$tag.refused]} { continue }
  puts "RUN $tag"
  set was {}
  set why {}
  foreach {prop value} [lrange $line 1 end] {
    lappend was $prop [get_property $prop $cell]
    if {[catch {set_property $prop $value $cell} message]} { set why $message; break }
  }
  if {$why eq {}} {
    if {[catch {write_bitstream -force $out/$tag.bit} message] || ![file exists $out/$tag.bit]} {
      set why $message
    }
  }
  if {$why ne {}} {
    set g [open $out/$tag.refused w]
    puts $g [string map {"\n" " "} $why]
    close $g
  }
  foreach {prop value} $was { catch {set_property $prop $value $cell} }
}
close $f
