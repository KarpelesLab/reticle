-- An array of multi-bit elements becomes an IR memory; the address is the
-- VHDL index minus the low bound.
library ieee;
use ieee.std_logic_1164.all;

entity memory is
  port (
    clk  : in  std_logic;
    we   : in  std_logic;
    addr : in  integer range 0 to 15;
    din  : in  std_logic_vector(7 downto 0);
    dout : out std_logic_vector(7 downto 0)
  );
end entity;

architecture rtl of memory is
  type ram_t is array (0 to 15) of std_logic_vector(7 downto 0);
  signal ram : ram_t;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if we = '1' then
        ram(addr) <= din;
      end if;
    end if;
  end process;

  dout <= ram(addr);
end architecture;
