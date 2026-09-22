-- A counter with an asynchronous reset: the `if rst elsif rising_edge`
-- idiom, which becomes a sequential process with a reset edge.
library ieee;
use ieee.std_logic_1164.all;

entity counter_async is
  port (
    clk : in  std_logic;
    rst : in  std_logic;
    q   : out integer range 0 to 255
  );
end entity;

architecture rtl of counter_async is
  signal cnt : integer range 0 to 255 := 0;
begin
  process (clk, rst)
  begin
    if rst = '1' then
      cnt <= 0;
    elsif rising_edge(clk) then
      cnt <= cnt + 1;
    end if;
  end process;

  q <= cnt;
end architecture;
