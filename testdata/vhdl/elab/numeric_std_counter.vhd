-- top: counter
-- An `ieee.numeric_std` counter: the increment folds statically, the
-- addition lowers to an IR `Add` with both operands resized to 8 bits.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity counter is
  port (
    clk  : in  std_logic;
    rst  : in  std_logic;
    en   : in  std_logic;
    q    : out std_logic_vector(7 downto 0);
    full : out std_logic
  );
end entity;

architecture rtl of counter is
  constant step : unsigned(7 downto 0) := to_unsigned(1, 8);
  signal cnt : unsigned(7 downto 0);
begin
  process (clk, rst) is
  begin
    if rst = '1' then
      cnt <= (others => '0');
    elsif rising_edge(clk) then
      if en = '1' then
        cnt <= cnt + step;
      end if;
    end if;
  end process;

  q    <= std_logic_vector(cnt);
  full <= '1' when cnt = 255 else '0';
end architecture;
