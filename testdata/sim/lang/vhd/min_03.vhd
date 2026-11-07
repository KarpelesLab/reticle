library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    variable u : unsigned(3 downto 0) := "1010";
    variable v : std_logic_vector(3 downto 0) := "1010";
  begin
    report to_string(u);
    report "u=" & to_string(u);
    report "v=" & to_string(v);
    report "h=" & to_hstring(v);
    wait;
  end process;
end architecture;
