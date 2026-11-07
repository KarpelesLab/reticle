library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  signal clk : std_logic := '0';
  signal a : unsigned(3 downto 0) := x"1";
  signal b : unsigned(3 downto 0) := x"2";
  signal sh : std_logic_vector(3 downto 0) := "0001";
  signal stop : boolean := false;
  signal n : integer := 0;
begin
  clk <= not clk after 5 ns when not stop else clk;

  regs : process (clk)
    variable tmp : unsigned(3 downto 0);
  begin
    if rising_edge(clk) then
      a <= b;
      b <= a;
      sh <= sh(2 downto 0) & sh(3);
      tmp := a + b;
      n <= to_integer(tmp);
    end if;
  end process;

  process
  begin
    wait until clk = '1';
    wait for 1 ns;
    if a = 2 and b = 1 then report "swap 1 ok"; else report "swap 1 bad"; end if;
    if sh = "0010" then report "shift 1 ok"; else report "shift 1 bad"; end if;
    if n = 3 then report "var in clocked ok"; else report "var in clocked bad"; end if;
    wait until clk = '1';
    wait for 1 ns;
    if a = 1 and b = 2 then report "swap 2 ok"; else report "swap 2 bad"; end if;
    if sh = "0100" then report "shift 2 ok"; else report "shift 2 bad"; end if;
    wait until clk = '1';
    wait until clk = '1';
    wait for 1 ns;
    if sh = "0001" then report "rotate full ok"; else report "rotate full bad"; end if;
    stop <= true;
    wait;
  end process;
end architecture;
