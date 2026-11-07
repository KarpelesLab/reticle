library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal s : std_logic := '1';
  signal n : integer := 4;
begin
  -- concurrent assertion: fires when n changes to a bad value
  assert n < 10 report "concurrent: n too large" severity warning;

  process
  begin
    assert true report "never shown";
    assert s = '1' report "never shown either" severity error;
    assert s = '0' report "s is not zero" severity note;
    assert s = '0' report "s is not zero (warning)" severity warning;
    assert s = '0' report "s is not zero (error)" severity error;
    assert s = '0' report "s is not zero (default severity)";
    assert false;
    assert false severity warning;
    report "plain report";
    report "report with note" severity note;
    report "report with warning" severity warning;
    report "report with error" severity error;
    report "";
    report "multipart message";
    report "quote "" inside";
    n <= 12;
    wait for 1 ns;
    n <= 3;
    wait for 1 ns;
    report "before failure";
    assert false report "fatal failure here" severity failure;
    report "after failure (must not print)";
    wait;
  end process;
end architecture;
