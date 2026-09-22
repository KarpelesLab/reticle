-- generic: WIDTH=4
-- Generics of several classes; `WIDTH` is overridden from the command
-- line, so the module is named after the value that differs.
library ieee;
use ieee.std_logic_1164.all;

entity generics is
  generic (
    WIDTH  : positive := 8;
    DEPTH  : natural  := 16;
    NAME   : string   := "gen";
    DEBUG  : boolean  := false
  );
  port (
    d : in  std_logic_vector(WIDTH - 1 downto 0);
    q : out std_logic_vector(WIDTH - 1 downto 0)
  );
end entity;

architecture rtl of generics is
  constant TOTAL : natural := WIDTH * DEPTH;
begin
  q <= d;

  assert TOTAL > 0
    report "empty " & NAME
    severity failure;
end architecture;
