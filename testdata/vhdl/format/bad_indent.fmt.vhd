library ieee;
use ieee.std_logic_1164.all;

entity ragged is
  generic (N : POSITIVE := 3; RESET_VALUE : std_logic := '0');
  port (
    clk : in  std_logic;
    rst : in  std_logic;
    en  : in  std_logic;
    q   : out std_logic_vector(N - 1 downto 0)
  );
end entity ragged;

architecture rtl of ragged is
  signal shifter : std_logic_vector(N - 1 downto 0);
  signal parity : std_logic;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        shifter <= (others => RESET_VALUE);
      elsif en = '1' then
        shifter <= shifter(N - 2 downto 0) & parity;
      end if;
    end if;
  end process;

  parity <= xor shifter;
  q      <= shifter;
end architecture rtl;
