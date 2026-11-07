library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal c0 : std_logic := '0';
  signal c1 : std_logic := '1';
  signal nf, nr : integer := 0;
begin
  pf : process (c0) begin if falling_edge(c0) then nf <= nf + 1; end if; end process;
  pr : process (c1) begin if rising_edge(c1) then nr <= nr + 1; end if; end process;
  process
  begin
    wait for 1 ns;
    if nf = 0 then report "no falling edge at t0 ok"; else report "falling edge fired at t0 on a signal initialised 0"; end if;
    if nr = 0 then report "no rising edge at t0 ok"; else report "rising edge fired at t0 on a signal initialised 1"; end if;
    wait;
  end process;
end architecture;
