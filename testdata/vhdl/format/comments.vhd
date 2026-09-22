-- A file header comment.
-- It runs over two lines.



library ieee;     -- the standard logic library
use ieee.std_logic_1164.all;
-- A comment between the context clause and the unit.

/* A delimited comment,
   spanning several lines,
   which must come back byte for byte. */
entity commented is
  -- before the generic clause
  generic (
    -- before an element
    W : positive := 8; -- after an element
    D : natural  := 0
    -- dangling inside the generic list
  );
  port (
    clk : in  std_logic; -- the clock
    -- a comment of its own
    d   : in  std_logic_vector(W - 1 downto 0);
    q   : out std_logic_vector(W - 1 downto 0)
  );
  -- dangling at the end of the entity
end entity commented;

architecture rtl of commented is
  -- a declaration comment
  signal r : std_logic_vector(W - 1 downto 0); /* a delimited trailing comment */

  -- after a blank line
  signal s : std_logic;
begin

  -- a comment before the process
  main : process (clk)
    -- inside the declarative part
    variable v : integer; -- trailing on a variable
  begin
    -- first thing in the body
    if rising_edge(clk) then
      r <= d; -- the interesting bit
      -- between two statements
      s <= r(0);
    end if;
    -- dangling at the end of the process body
  end process main;

  q <= r;
  -- the very last comment of the file
end architecture rtl;
-- after the architecture
