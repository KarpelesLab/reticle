-- A synchronous counter over an integer subtype.
library ieee;
use ieee.std_logic_1164.all;

entity counter is
  generic (WIDTH : positive := 8);
  port (
    clk  : in  std_logic;
    rst  : in  std_logic;
    en   : in  std_logic;
    full : out std_logic
  );
end entity counter;

architecture rtl of counter is
  constant TOP : natural := 2 ** WIDTH - 1;
  signal cnt   : natural range 0 to 255 := 0;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        cnt <= 0;
      elsif en = '1' then
        if cnt = 255 then
          cnt <= 0;
        else
          cnt <= cnt + 1;
        end if;
      end if;
    end if;
  end process;

  full <= '1' when cnt = 255 else '0';
end architecture rtl;
