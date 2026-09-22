library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity mux is
  generic (W : positive := 4; LATENCY : natural := 0);
  port (
    clk        : in  std_logic;
    sel        : in  std_logic_vector(1 downto 0);
    a, b, c, d : in  std_logic_vector(W - 1 downto 0);
    y          : out std_logic_vector(W - 1 downto 0)
  );
end entity mux;
architecture rtl of mux is
  signal q : std_logic_vector(W - 1 downto 0);
begin
  with sel select q <= a when "00", b when "01", c when "10", d when others;
  reg : process (clk)
  begin
    if rising_edge(clk) then
      y <= q;
    end if;
  end process reg;
end architecture rtl;
