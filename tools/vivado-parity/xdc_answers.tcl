# What Vivado makes of one XDC file: `vivado -mode batch -source xdc_answers.tcl
# -tclargs <dir> <case>`, from a scratch directory. gen_xdc.py runs it for each
# case and keeps the answer. Read after synthesis, so that a cell a constraint
# names exists and is read once rather than at each of synthesis's two passes.
lassign $argv dir case
create_project -in_memory -part xc7a100tftg256-1
read_verilog $dir/design.v
synth_design -top design_top -part xc7a100tftg256-1 -flatten_hierarchy none
puts "ANSWER begin"
read_xdc $dir/$case.xdc
puts "ANSWER end"
set_msg_config -severity {INFO} -suppress
foreach p [lsort -dictionary [get_ports *]] {
  puts "ANSWER port $p [get_property DIRECTION $p]\
    pin=[get_property PACKAGE_PIN $p]\
    iostandard=[get_property IOSTANDARD $p]\
    drive=[get_property DRIVE $p]\
    slew=[get_property SLEW $p]\
    pulltype=[get_property PULLTYPE $p]"
}
foreach c [lsort -dictionary [get_clocks -quiet *]] {
  puts "ANSWER clock $c\
    period=[get_property PERIOD $c]\
    waveform=[join [get_property WAVEFORM $c] ,]\
    generated=[get_property IS_GENERATED $c]\
    user=[get_property IS_USER_GENERATED $c]\
    virtual=[get_property IS_VIRTUAL $c]\
    sources=[join [lsort -dictionary [get_property SOURCE_PINS $c]] ,]"
}
foreach b [lsort -dictionary [get_pblocks -quiet *]] {
  puts "ANSWER pblock $b\
    ranges=[join [get_property GRID_RANGES $b] ,]\
    cells=[join [lsort -dictionary [get_cells -quiet -of_objects $b]] ,]"
}
# The tile each corner of a pblock is in, which is what Reticle makes of it.
foreach b [lsort -dictionary [get_pblocks -quiet *]] {
  foreach range [get_property GRID_RANGES $b] {
    foreach site [split $range :] {
      if {[string match SLICE_* $site]} {
        puts "ANSWER tile $site [get_tiles -of_objects [get_sites $site]]"
      }
    }
  }
}
foreach c [lsort -dictionary [get_cells -hierarchical -quiet *_reg*]] {
  set loc [get_property LOC $c]
  set bel [get_property BEL $c]
  if {$loc ne "" || $bel ne ""} { puts "ANSWER place $c loc=$loc bel=$bel" }
}
foreach p [lsort [list_property [current_design]]] {
  set v [get_property $p [current_design]]
  if {[string match BITSTREAM.* $p] || $p in {CFGBVS CONFIG_VOLTAGE CONFIG_MODE}} {
    if {$v ne "" && $v ne "NONE" && $v ne "DEFAULT"} { puts "ANSWER config $p=$v" }
  }
}
# What it kept of the path constraints, each object as Vivado resolved it: its
# own rewrite of the file, run through procedures that print their arguments.
# A bare name after -from is what Vivado's "implicit search" makes of it, which
# looks for a clock, then a cell, then a port, then a pin.
proc named {value} {
  if {[llength $value] == 0} { return "" }
  if {![catch {get_property CLASS [lindex $value 0]} class] && $class ne ""} {
    set out {}
    foreach o $value { lappend out "[get_property CLASS $o]:$o" }
    return [join [lsort -dictionary $out] ,]
  }
  set out {}
  foreach name $value {
    foreach {class query} {clock get_clocks cell get_cells port get_ports pin get_pins} {
      set found [$query -quiet $name]
      if {[llength $found]} {
        foreach o $found { lappend out "$class:$o" }
        break
      }
    }
  }
  return [join [lsort -dictionary $out] ,]
}
proc said {command arguments} {
  set out [list]
  set wanted 0
  set literal 0
  foreach a $arguments {
    if {$wanted} {
      lappend out [named $a]
      set wanted 0
    } elseif {$literal} {
      lappend out $a
      set literal 0
    } elseif {$a in {-from -to -through -group -clock}} {
      lappend out $a
      set wanted 1
    } elseif {$a eq "-name"} {
      lappend out $a
      set literal 1
    } elseif {[string match -* $a] || [string is double -strict $a]} {
      lappend out $a
    } else {
      lappend out [named $a]
    }
  }
  puts "ANSWER timing $command [join $out " "]"
}
write_xdc -force -no_fixed_only constraints_out.xdc
set commands {set_false_path set_multicycle_path set_max_delay set_min_delay set_input_delay set_output_delay set_clock_groups}
foreach c $commands {
  rename $c real_$c
  proc $c {args} "said $c \$args"
}
set f [open constraints_out.xdc]
foreach line [split [read $f] "\n"] {
  if {[lindex [split $line] 0] in $commands} { eval $line }
}
close $f
