library ieee;
use ieee.std_logic_1164.all;

entity first
  port (a : in bit);
end entity first;

this is not a design unit at all;

architecture rtl of first is
begin
  y <= a;
end architecture rtl;

package p is
  constant K : integer := 1;
end package p
