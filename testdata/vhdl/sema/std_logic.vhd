-- The bundled ieee.std_logic_1164 package in use.
library ieee;
use ieee.std_logic_1164.all;

entity slv_user is
  port (
    clk  : in  std_logic;
    a, b : in  std_logic_vector(7 downto 0);
    sel  : in  std_logic;
    y    : out std_logic_vector(7 downto 0);
    z    : out std_logic;
    ok   : out boolean
  );
end entity;

architecture rtl of slv_user is
  constant MASK : std_logic_vector(7 downto 0) := x"F0";
  signal t      : std_logic_vector(7 downto 0);
  signal e      : std_ulogic;
begin
  t <= (a and MASK) or (b and not MASK);
  y <= t xor a;
  z <= sel;
  e <= to_stdulogic('1');

  ok <= is_x(a) = false;

  edge : process (clk)
  begin
    if rising_edge(clk) then
      z <= a(0);
    elsif falling_edge(clk) then
      z <= b(0);
    end if;
  end process;
end architecture;
