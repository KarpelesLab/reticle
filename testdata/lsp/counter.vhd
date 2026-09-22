-- A counter entity and its architecture, for the language server's VHDL
-- session.
library ieee;
use ieee.std_logic_1164.all;

entity counter is
  port (
    clk : in  std_logic;
    rst : in  std_logic;
    q   : out std_logic_vector(7 downto 0)
  );
end entity counter;

architecture rtl of counter is
  signal count : std_logic_vector(7 downto 0);
begin
  tick : process (clk, rst) is
  begin
    if rst = '1' then
      count <= (others => '0');
    elsif clk = '1' then
      count <= count;
    end if;
  end process tick;

  q <= count;
end architecture rtl;
