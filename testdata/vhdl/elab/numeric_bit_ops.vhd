-- top: bitops
-- `ieee.numeric_bit` over `bit_vector`. The lowering is the same core as
-- `ieee.numeric_std`: only the element type differs, and a lowered bit is
-- a lowered bit either way.
library ieee;
use ieee.numeric_bit.all;

entity bitops is
  port (
    a   : in  bit_vector(7 downto 0);
    b   : in  bit_vector(7 downto 0);
    sum : out bit_vector(7 downto 0);
    dif : out bit_vector(7 downto 0);
    prd : out bit_vector(15 downto 0);
    shr : out bit_vector(7 downto 0);
    lss : out bit
  );
end entity;

architecture rtl of bitops is
  signal ua : unsigned(7 downto 0);
  signal sb : signed(7 downto 0);
begin
  ua <= unsigned(a);
  sb <= signed(b);

  sum <= bit_vector(ua + 1);
  dif <= bit_vector(signed(a) - sb);
  prd <= bit_vector(ua * unsigned(b));
  shr <= bit_vector(shift_right(sb, 2));
  lss <= '1' when sb < 0 else '0';
end architecture;
