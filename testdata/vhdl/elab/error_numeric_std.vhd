-- `ieee.numeric_std` is not bundled yet: naming it gives one clear
-- diagnostic rather than a cascade of unknown identifiers.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity error_numeric_std is
  port (
    a : in  std_logic_vector(7 downto 0);
    q : out std_logic_vector(7 downto 0)
  );
end entity;

architecture rtl of error_numeric_std is
begin
  q <= a;
end architecture;
