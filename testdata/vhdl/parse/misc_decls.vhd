library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;

package misc_pkg is
  shared variable counter : integer := 0;
  variable plain_var : integer;
  signal s1, s2 : bit register;
  signal b : bit bus := '0';
  file log_file : text open write_mode is "log.txt";
  file in_file : text is "in.txt";
  file bare : text;
  group pin_pair is (signal, signal);
  group timing is (label <>);
  group p1 : pin_pair (a, b);
  group t1 : timing (u0, u1, u2);
  disconnect s1, s2 : bit after 5 ns;
  disconnect others : bit after 1 ns;
  disconnect all : bit after 0 ns;
  use work.other_pkg.all;
  use work.other_pkg.item, work.third_pkg."+";

  package nested is
    constant K : integer := 1;
  end package nested;

  package body nested is
  end package body nested;

  package nested_inst is new work.gen_pkg generic map (N => 4);
end package misc_pkg;

architecture a of e is
  component c
    port (x : in bit);
  end component;
  for u0 : c use entity work.c_impl(rtl);
  for u1, u2 : c use configuration work.c_cfg generic map (N => 1) port map (x => x);
  for others : c use open;
  for all : c use entity work.c_impl port map (x => x);
  for u3 : c use entity work.c_impl;
  end for;
begin
end architecture;
