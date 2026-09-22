-- Two drivers of one resolved signal: each moves onto its own net and the
-- signal is driven by the resolution logic.
library ieee;
use ieee.std_logic_1164.all;

entity resolution is
  port (
    a, b   : in  std_logic;
    ea, eb : in  std_logic;
    bus_o  : out std_logic
  );
end entity;

architecture rtl of resolution is
  signal net_s : std_logic;
begin
  net_s <= a when ea = '1' else 'Z';
  net_s <= b when eb = '1' else 'Z';

  bus_o <= net_s;
end architecture;
