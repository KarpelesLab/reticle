# Writes one bitstream per line of a list, from the checkpoint bitstream_base.tcl
# kept. A line is `<name> NAME=VALUE [NAME=VALUE ...]`.
#   vivado -mode batch -source bitstream_options.tcl -tclargs <dcp> <list> <out>
# A refusal is an answer too: <name>.refused has why.
lassign $argv dcp list out
file mkdir $out
cd $out
open_checkpoint $dcp
set in [open $list r]
set lines [split [read $in] "\n"]
close $in
foreach line $lines {
  if {[llength $line] < 2} continue
  set name [lindex $line 0]
  set sets [lrange $line 1 end]
  set failed ""
  foreach kv $sets {
    set eq [string first = $kv]
    set p [string range $kv 0 [expr {$eq - 1}]]
    set v [string range $kv [expr {$eq + 1}] end]
    if {[catch {set_property $p $v [current_design]} message]} { set failed "set $kv: $message" }
  }
  if {$failed eq "" && [catch {write_bitstream -force $name.bit} message]} { set failed $message }
  if {$failed ne ""} {
    set f [open $name.refused w]
    puts $f $failed
    close $f
    file delete $name.bit
  }
  foreach kv $sets {
    set p [string range $kv 0 [expr {[string first = $kv] - 1}]]
    catch {reset_property $p [current_design]}
  }
}
