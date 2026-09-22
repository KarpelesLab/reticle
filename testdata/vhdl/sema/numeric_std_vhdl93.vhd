-- sema: vhdl93
-- The bundled packages are written in VHDL-2008 and are always analysed
-- that way, but a VHDL-93 design must still see and use them. The 2008
-- spellings (`u_unsigned`, the matching operators) are simply not
-- mentioned here.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.std_logic_textio.all;
use std.textio.all;

entity counter93 is
  port (
    clk : in  std_logic;
    q   : out std_logic_vector(7 downto 0)
  );
end entity;

architecture rtl of counter93 is
  constant step : unsigned(7 downto 0) := to_unsigned(1, 8);
  signal cnt : unsigned(7 downto 0);
begin
  process (clk)
  begin
    if rising_edge(clk) then
      cnt <= cnt + step;
    end if;
  end process;

  q <= std_logic_vector(cnt);

  logging : process (cnt)
    variable l : line;
  begin
    write(l, string'("count "));
    hwrite(l, std_logic_vector(cnt));
    writeline(output, l);
  end process;
end architecture;
