library ieee;
use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
begin
  process begin
    report "a";
    wait for 1 ns;
    std.env.finish;
  end process;
end architecture;
