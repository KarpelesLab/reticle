-- Attribute specifications on an entity and on signals travel into the
-- IR's attribute maps.
library ieee;
use ieee.std_logic_1164.all;

entity attributes is
  port (
    clk : in  std_logic;
    d   : in  std_logic;
    q   : out std_logic
  );
end entity;

architecture rtl of attributes is
  attribute keep      : boolean;
  attribute ram_style : string;

  signal stage : std_logic;
  attribute keep of stage : signal is true;
  attribute ram_style of stage : signal is "distributed";
begin
  process (clk)
  begin
    if rising_edge(clk) then
      stage <= d;
      q     <= stage;
    end if;
  end process;
end architecture;
