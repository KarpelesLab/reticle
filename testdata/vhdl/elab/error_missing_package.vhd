-- A package Reticle does not bundle: naming it gives one clear diagnostic
-- rather than a cascade of unknown identifiers.
library ieee;
use ieee.std_logic_1164.all;
use ieee.fixed_pkg.all;

entity error_missing_package is
  port (
    a : in  std_logic_vector(7 downto 0);
    q : out std_logic_vector(7 downto 0)
  );
end entity;

architecture rtl of error_missing_package is
begin
  q <= a;
end architecture;
