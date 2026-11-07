-- Objects whose subtype leaves the bounds open — string generics and
-- constants, unconstrained parameters and results, a port sized by a
-- generic's range — held to what Vivado 2026.1's xsim printed for this
-- file (unconstrained_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;

entity sized is
  generic (INIT : std_logic_vector := "1010"; NAME : string := "sized");
  port (iv : out std_logic_vector(INIT'range); nl : out integer);
end entity;

architecture rtl of sized is
begin
  iv <= INIT;
  nl <= NAME'length;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity tb is end entity;

architecture sim of tb is
  constant G : string := "hello";
  signal iv : std_logic_vector(5 downto 0);
  signal nl : integer;
  signal a : std_logic_vector(7 downto 0) := x"00";
  signal s : std_logic_vector(7 downto 0) := x"F1";

  function popcount(v : std_logic_vector) return natural is
    variable c : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then c := c + 1; end if;
    end loop;
    return c;
  end function;

  function reverse(v : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(v'reverse_range);
  begin
    for i in v'range loop
      r(i) := v(i);
    end loop;
    return r;
  end function;

  procedure drive(signal t : out std_logic_vector; v : in std_logic_vector) is
  begin
    t <= v;
  end procedure;
begin
  u : entity work.sized generic map (INIT => "110011", NAME => "unit") port map (iv, nl);

  process
  begin
    report "G'length=" & integer'image(G'length) & " left=" & integer'image(G'left)
      & " right=" & integer'image(G'right);
    report "popcount " & integer'image(popcount(s)) & " " & integer'image(popcount("0110"));
    if reverse("1100") = "0011" and reverse(x"01") = x"80" then report "reverse"; end if;
    drive(a, x"5A");
    wait for 1 ns;
    report "a=" & to_hstring(a) & " iv=" & to_string(iv) & " nl=" & integer'image(nl);
    wait;
  end process;
end architecture;
