library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- A synchronous up-counter with enable and synchronous reset.
entity counter is
  generic (
    WIDTH : positive := 8;
    T_CLK : time     := 10 ns
  );
  port (
    clk    : in  std_logic;
    rst    : in  std_logic;
    en     : in  std_logic;
    count  : out std_logic_vector(WIDTH - 1 downto 0);
    wrap   : out std_logic
  );
end entity counter;

architecture rtl of counter is
  signal cnt : unsigned(WIDTH - 1 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        cnt <= (others => '0');
      elsif en = '1' then
        cnt <= cnt + 1;
      end if;
    end if;
  end process;

  count <= std_logic_vector(cnt);
  wrap  <= '1' when cnt = 2 ** WIDTH - 1 and en = '1' else '0';

  assert WIDTH <= 32
    report "WIDTH " & integer'image(WIDTH) & " exceeds 32"
    severity failure;
end architecture rtl;
