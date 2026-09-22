-- A counter with a synchronous reset: the clocked `if rising_edge` idiom.
library ieee;
use ieee.std_logic_1164.all;

entity counter_sync is
  generic (WIDTH : positive := 8);
  port (
    clk : in  std_logic;
    rst : in  std_logic;
    en  : in  std_logic;
    q   : out std_logic_vector(WIDTH - 1 downto 0)
  );
end entity;

architecture rtl of counter_sync is
  signal cnt : std_logic_vector(WIDTH - 1 downto 0);
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        cnt <= (others => '0');
      elsif en = '1' then
        cnt <= cnt xor "00000001";
      end if;
    end if;
  end process;

  q <= cnt;
end architecture;
