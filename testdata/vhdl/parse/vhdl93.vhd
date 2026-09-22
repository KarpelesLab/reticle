-- parse: vhdl93
-- VHDL-93 mode: 2008 words are identifiers, and 2008 constructs warn.
library ieee;
use ieee.std_logic_1164.all;

entity legacy is
  port (clk : in std_logic; default : in std_logic; force : out std_logic);
end legacy;

architecture rtl of legacy is
  signal context : std_logic;
  signal parameter : integer;
begin
  process (clk)
  begin
    if clk'event and clk = '1' then
      context <= default;
    end if;
  end process;

  force <= context;

  p2 : process (all)
  begin
    parameter <= 1;
  end process p2;

  g : if parameter = 1 generate
    force <= '1';
  else generate
    force <= '0';
  end generate g;
end rtl;
