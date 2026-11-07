-- Inertial and transport delay, held to what Vivado 2026.1's xsim printed
-- for this file (inertial_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;

entity tb is end entity;

architecture sim of tb is
  signal src, inert, trans : std_logic := '0';
  signal w, t : integer := 0;
begin
  inert <= src after 3 ns;
  trans <= transport src after 3 ns;

  process
  begin
    wait for 1 ns;
    src <= '1';
    wait for 2 ns;
    src <= '0';
    wait for 1 ns;
    report "at 4 ns inert=" & std_logic'image(inert) & " trans=" & std_logic'image(trans);
    wait for 2 ns;
    report "at 6 ns inert=" & std_logic'image(inert) & " trans=" & std_logic'image(trans);
    src <= '1';
    wait for 4 ns;
    report "a long pulse passes: inert=" & std_logic'image(inert);
    w <= 10 after 2 ns;
    t <= transport 10 after 2 ns;
    wait for 1 ns;
    w <= 20 after 3 ns;
    t <= transport 20 after 3 ns;
    wait for 1500 ps;
    report "w=" & integer'image(w) & " t=" & integer'image(t);
    wait for 2 ns;
    report "w=" & integer'image(w) & " t=" & integer'image(t);
    w <= 1 after 1 ns, 2 after 2 ns, 3 after 3 ns;
    wait for 4 ns;
    report "waveform w=" & integer'image(w);
    wait;
  end process;
end architecture;
