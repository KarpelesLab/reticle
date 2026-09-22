-- top: tb
-- A testbench: a free-running process with `wait for`, `report` and an
-- assertion, driving the unit under test.
library ieee;
use ieee.std_logic_1164.all;

entity dut is
  port (
    clk : in  std_logic;
    d   : in  std_logic;
    q   : out std_logic
  );
end entity;

architecture rtl of dut is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= d;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal clk : std_logic := '0';
  signal d   : std_logic := '0';
  signal q   : std_logic;
begin
  uut : entity work.dut port map (clk => clk, d => d, q => q);

  clocking : process
  begin
    for i in 0 to 3 loop
      wait for 5 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
    end loop;
    wait;
  end process;

  stimulus : process
  begin
    report "start";
    d <= '1';
    wait for 12 ns;
    assert q = '1' report "q did not follow d" severity error;
    report "done";
    wait;
  end process;
end architecture;
