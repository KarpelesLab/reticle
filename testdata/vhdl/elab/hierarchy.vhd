-- top: top
-- A two-level hierarchy bound through a component declaration: the
-- default binding finds the entity of the same name in `work`.
library ieee;
use ieee.std_logic_1164.all;

entity adder is
  generic (WIDTH : positive := 4);
  port (
    a, b : in  std_logic_vector(WIDTH - 1 downto 0);
    y    : out std_logic_vector(WIDTH - 1 downto 0)
  );
end entity;

architecture rtl of adder is
begin
  y <= a xor b;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity top is
  port (
    p, q : in  std_logic_vector(3 downto 0);
    r    : in  std_logic_vector(7 downto 0);
    s    : out std_logic_vector(3 downto 0);
    t    : out std_logic_vector(7 downto 0)
  );
end entity;

architecture rtl of top is
  component adder is
    generic (WIDTH : positive := 4);
    port (
      a, b : in  std_logic_vector(WIDTH - 1 downto 0);
      y    : out std_logic_vector(WIDTH - 1 downto 0)
    );
  end component;
begin
  u_small : component adder
    generic map (WIDTH => 4)
    port map (a => p, b => q, y => s);

  u_wide : component adder
    generic map (WIDTH => 8)
    port map (a => r, b => r, y => t);
end architecture;
