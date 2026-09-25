-- The attributes that stand for a subtype or a range, in the three places
-- Colibri writes them: `x'subtype` as a declaration's subtype indication,
-- `x'range` as the choice of an aggregate, and `x'range` as the discrete
-- range of a slice. `'left` and `'length` of a port whose width came from a
-- generic are here too, since nothing static can be folded for them.
--
-- Found by the Colibri corpus; see docs/vhdl-corpus.md.
library ieee;
use ieee.std_logic_1164.all;

entity subtype_and_range_attributes is
  generic (WIDTH : positive := 6);
  port (
    clk    : in  std_logic;
    data_i : in  std_logic_vector(WIDTH - 1 downto 0);
    wide_o : out std_logic_vector(2 * WIDTH - 1 downto 0);
    all_o  : out std_logic;
    top_o  : out std_logic
  );
end entity;

architecture rtl of subtype_and_range_attributes is

  -- `'subtype` takes the port's subtype, bounds included.
  signal reg  : data_i'subtype;
  signal ones : data_i'subtype := (data_i'range => '1');

begin

  reg <= data_i;

  -- An aggregate whose only choice is a range attribute, compared against
  -- the object it came from.
  all_o <= '1' when reg = (reg'range => '1') else '0';

  -- A slice whose discrete range is another object's range, and one whose
  -- bound is an attribute of an object sized by a generic.
  wide_o(data_i'range) <= reg;
  wide_o(2 * WIDTH - 1 downto data_i'length) <= ones;
  top_o                                      <= reg(reg'left);

end architecture;
