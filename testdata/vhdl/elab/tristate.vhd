-- A single driver that is conditionally high impedance becomes a
-- `tristate` cell rather than a mux against `'Z'`.
library ieee;
use ieee.std_logic_1164.all;

entity tristate is
  port (
    d  : in    std_logic_vector(7 downto 0);
    en : in    std_logic;
    io : inout std_logic_vector(7 downto 0)
  );
end entity;

architecture rtl of tristate is
begin
  io <= d when en = '1' else (others => 'Z');
end architecture;
