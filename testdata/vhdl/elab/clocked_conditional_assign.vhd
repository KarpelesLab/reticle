-- `q <= d when rising_edge(clk);` is a register written as one concurrent
-- statement, and the whole of Colibri's two-process style rests on it: a
-- combinational process computes the next value and one of these clocks it.
-- It must lower to the same process the `if rising_edge(clk)` idiom gives.
--
-- Found by the Colibri corpus; see docs/vhdl-corpus.md.
library ieee;
use ieee.std_logic_1164.all;

entity clocked_conditional_assign is
  port (
    clk : in  std_logic;
    d   : in  std_logic_vector(3 downto 0);
    q   : out std_logic_vector(3 downto 0);
    n   : out std_logic_vector(3 downto 0)
  );
end entity;

architecture rtl of clocked_conditional_assign is
  signal reg : std_logic_vector(3 downto 0);
  signal cmb : std_logic_vector(3 downto 0);
begin

  cmb <= not d;
  reg <= cmb when rising_edge(clk);

  -- The same on a falling edge, to a port.
  n <= reg when falling_edge(clk);

  q <= reg;

end architecture;
