library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal a, b, c, d, e, f, g, h : std_logic;
  signal bus8 : std_logic_vector(3 downto 0);
  signal da, db : std_logic := 'Z';
begin
  -- multiple concurrent drivers
  a <= '0'; a <= '1';            -- X
  b <= '1'; b <= 'Z';            -- 1
  c <= 'H'; c <= 'L';            -- W
  d <= 'L'; d <= 'Z';            -- L
  e <= 'H'; e <= '0';            -- 0
  f <= 'W'; f <= 'H';            -- W
  g <= 'Z'; g <= 'Z';            -- Z
  h <= 'U'; h <= '1';            -- U
  bus8 <= "10ZH";
  bus8 <= "Z0LZ";
  -- process drivers
  p1 : process
  begin
    da <= '1';
    wait for 2 ns;
    da <= 'Z';
    wait for 2 ns;
    da <= '0';
    wait;
  end process;
  p2 : process
  begin
    db <= 'Z';
    wait for 1 ns;
    db <= 'Z';
    wait;
  end process;
  chk : process
  begin
    wait for 1 ns;
    report "a=" & std_logic'image(a);
    report "b=" & std_logic'image(b);
    report "c=" & std_logic'image(c);
    report "d=" & std_logic'image(d);
    report "e=" & std_logic'image(e);
    report "f=" & std_logic'image(f);
    report "g=" & std_logic'image(g);
    report "h=" & std_logic'image(h);
    report "bus=" & to_string(bus8);
    report "da t1=" & std_logic'image(da);
    wait for 2 ns;
    report "da t3=" & std_logic'image(da);
    wait for 2 ns;
    report "da t5=" & std_logic'image(da);
    report "to_x01(H)=" & std_logic'image(to_x01('H'));
    report "to_x01(W)=" & std_logic'image(to_x01('W'));
    report "to_ux01(L)=" & std_logic'image(to_ux01('L'));
    report "to_01 Z=" & to_string(to_01(std_logic_vector'("1Z0H"), '1'));
    report "is_x(10X)=" & boolean'image(is_x(std_logic_vector'("10X")));
    report "is_x(10H)=" & boolean'image(is_x(std_logic_vector'("10H")));
    report "res H and L=" & std_logic'image('H' and 'L');
    report "res H or Z=" & std_logic'image('H' or 'Z');
    report "res W xor 1=" & std_logic'image('W' xor '1');
    report "not U=" & std_logic'image(not 'U');
    report "not Z=" & std_logic'image(not 'Z');
    report "rising_edge type=" & boolean'image(std_logic'('H') = 'H');
    wait;
  end process;
end architecture;
