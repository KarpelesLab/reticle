-- top: not_static
-- The boundary of the native implementations. Almost everything the
-- arithmetic packages declare lowers to an IR operator, but a rotate by
-- an amount that is not known at elaboration, a search for a bit
-- position, and any `ieee.math_real` call on a signal do not, and each
-- says so rather than failing to find a body to inline.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.math_real.all;

entity not_static is
  port (
    a     : in  std_logic_vector(7 downto 0);
    n     : in  std_logic_vector(2 downto 0);
    rot   : out std_logic_vector(7 downto 0);
    first : out std_logic_vector(7 downto 0);
    root  : out std_logic_vector(7 downto 0)
  );
end entity;

architecture rtl of not_static is
begin
  rot   <= std_logic_vector(rotate_left(unsigned(a), to_integer(unsigned(n))));
  first <= std_logic_vector(to_unsigned(find_leftmost(unsigned(a), '1'), 8));
  root  <= std_logic_vector(to_unsigned(integer(sqrt(real(to_integer(unsigned(a))))), 8));
end architecture;
