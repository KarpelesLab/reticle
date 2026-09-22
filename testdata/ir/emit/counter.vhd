library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity counter is
  generic (
    WIDTH : integer := 8
  );
  port (
    clk : in std_logic;
    rst : in std_logic;
    en : in std_logic;
    q : out unsigned(7 downto 0)
  );
  attribute keep : integer;
  attribute keep of q : signal is 1;
end entity counter;

architecture rtl of counter is
begin
  count : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        q <= unsigned'("00000000");
      elsif en = '1' then
        q <= (q + unsigned'("00000001"));
      end if;
    end if;
  end process;
end architecture rtl;
