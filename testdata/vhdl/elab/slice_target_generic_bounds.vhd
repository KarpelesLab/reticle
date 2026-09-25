-- A slice assigned to, whose bounds are not locally static: they follow
-- from a generic, or from an attribute of an object sized by one. All three
-- of these assignments must appear in the module; the second and the third
-- were once dropped in silence, leaving a port undriven and saying nothing.
--
-- Found by the Colibri corpus; see docs/vhdl-corpus.md.
library ieee;
use ieee.std_logic_1164.all;

entity slice_target_generic_bounds is
  generic (WIDTH : positive := 6);
  port (
    data_i : in  std_logic_vector(WIDTH - 1 downto 0);
    a_o    : out std_logic_vector(2 * WIDTH - 1 downto 0);
    b_o    : out std_logic_vector(2 * WIDTH - 1 downto 0);
    c_o    : out std_logic_vector(2 * WIDTH - 1 downto 0)
  );
end entity;

architecture rtl of slice_target_generic_bounds is
begin
  a_o(11 downto 6)                        <= data_i;
  b_o(2 * WIDTH - 1 downto WIDTH)         <= data_i;
  c_o(2 * WIDTH - 1 downto data_i'length) <= data_i;
end architecture;
