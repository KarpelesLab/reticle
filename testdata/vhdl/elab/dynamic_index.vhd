-- A dynamic index into an array of single-bit elements is an indexed
-- slice of the wide net; a slice with static bounds is a constant one.
library ieee;
use ieee.std_logic_1164.all;

entity dynamic_index is
  port (
    d   : in  std_logic_vector(7 downto 0);
    u   : in  std_logic_vector(0 to 7);
    sel : in  integer range 0 to 7;
    bit_d : out std_logic;
    bit_u : out std_logic;
    nib   : out std_logic_vector(3 downto 0)
  );
end entity;

architecture rtl of dynamic_index is
begin
  bit_d <= d(sel);
  bit_u <= u(sel);
  nib   <= d(7 downto 4);
end architecture;
