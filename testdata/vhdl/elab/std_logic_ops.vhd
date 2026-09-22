-- The operators and conversions of `ieee.std_logic_1164` lower to IR
-- nodes rather than to their nine-state lookup tables.
library ieee;
use ieee.std_logic_1164.all;

entity std_logic_ops is
  port (
    a, b : in  std_logic_vector(7 downto 0);
    s    : in  std_logic;
    y    : out std_logic_vector(7 downto 0);
    z    : out std_logic;
    v    : out bit_vector(7 downto 0);
    u    : out boolean
  );
end entity;

architecture rtl of std_logic_ops is
  constant MASK : std_logic_vector(7 downto 0) := x"F0";
begin
  y <= (a and MASK) or (b and not MASK);
  z <= s xor a(0);
  v <= to_bitvector(a);
  u <= is_x(b);
end architecture;
