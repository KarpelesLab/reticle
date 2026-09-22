-- top: convert
-- Mixing `to_integer` and `to_unsigned`: the conversions are `Resize`
-- nodes, the static ones fold away entirely, and an index computed
-- through `to_integer` addresses a table.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity convert is
  generic (width : positive := 8);
  port (
    sel   : in  std_logic_vector(2 downto 0);
    step  : in  std_logic_vector(width - 1 downto 0);
    base  : out std_logic_vector(width - 1 downto 0);
    sum   : out std_logic_vector(width - 1 downto 0);
    small : out std_logic_vector(3 downto 0);
    big   : out std_logic_vector(15 downto 0)
  );
end entity;

architecture rtl of convert is
  -- Folded by the analyser: `to_unsigned` of a literal is a constant.
  constant seed  : unsigned(width - 1 downto 0) := to_unsigned(37, width);
  constant limit : integer := to_integer(seed) + 3;

  signal index : integer range 0 to 7;
  signal acc   : unsigned(width - 1 downto 0);
begin
  index <= to_integer(unsigned(sel));
  acc   <= seed + unsigned(step);

  base  <= std_logic_vector(seed);
  sum   <= std_logic_vector(acc);
  small <= std_logic_vector(resize(acc, 4));
  big   <= std_logic_vector(to_unsigned(limit * index, 16));
end architecture;
