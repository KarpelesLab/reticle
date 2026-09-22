-- top: alu
-- A signed ALU over `ieee.numeric_std`: every arithmetic operator, the
-- shifts and a comparison, each lowering to one IR node with explicit
-- resizes on the operands.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity alu is
  port (
    op    : in  std_logic_vector(3 downto 0);
    a     : in  std_logic_vector(7 downto 0);
    b     : in  std_logic_vector(7 downto 0);
    y     : out std_logic_vector(7 downto 0);
    wide  : out std_logic_vector(15 downto 0);
    lt    : out std_logic
  );
end entity;

architecture rtl of alu is
  signal sa : signed(7 downto 0);
  signal sb : signed(7 downto 0);
  signal r  : signed(7 downto 0);
begin
  sa <= signed(a);
  sb <= signed(b);

  with op select r <=
    sa + sb                       when x"0",
    sa - sb                       when x"1",
    -sa                           when x"2",
    abs(sa)                       when x"3",
    sa / sb                       when x"4",
    sa rem sb                     when x"5",
    sa mod sb                     when x"6",
    shift_left(sa, 2)             when x"7",
    shift_right(sa, 2)            when x"8",
    rotate_left(sa, 3)            when x"9",
    sa sll 1                      when x"a",
    sa srl 1                      when x"b",
    resize(sa(3 downto 0), 8)     when x"c",
    sa + 1                        when x"d",
    maximum(sa, sb)               when x"e",
    minimum(sa, sb)               when others;

  y    <= std_logic_vector(r);
  wide <= std_logic_vector(sa * sb);
  lt   <= '1' when sa < sb else '0';
end architecture;
