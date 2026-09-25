-- A conditional assignment with no `else` and no clock edge: the signal
-- keeps its previous value when the condition is false, which is a latch.
-- Reticle has no IR form for one, and used to assign zero instead, quietly
-- building a different design from the one written. It must say so.
--
-- Found by the Colibri corpus; see docs/vhdl-corpus.md.
library ieee;
use ieee.std_logic_1164.all;

entity error_conditional_no_else is
  port (
    en : in  std_logic;
    d  : in  std_logic_vector(3 downto 0);
    q  : out std_logic_vector(3 downto 0)
  );
end entity;

architecture rtl of error_conditional_no_else is
begin
  q <= d when en = '1';
end architecture;
