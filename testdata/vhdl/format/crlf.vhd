library ieee;
use ieee.std_logic_1164.all;

-- A file with CRLF line endings.
entity crlf is
  port (a : in std_logic;
        y : out std_logic);
end entity crlf;

architecture rtl of crlf is
begin
  y <= not a; -- trailing comment
end architecture rtl;
