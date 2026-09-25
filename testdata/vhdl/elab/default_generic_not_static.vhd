-- top: wrapper
--
-- A generic and a port whose defaults are not locally static: `WIDTH` is
-- computed from another generic, and `INIT`'s aggregate has no bounds until
-- the entity is instantiated. Both may be left out of a map all the same
-- (LRM 6.5.2), and were once reported as having no association at all
-- because no value could be folded for them.
--
-- Found by the Colibri corpus; see docs/vhdl-corpus.md.
library ieee;
use ieee.std_logic_1164.all;

package sizes is
  function double (arg : natural) return natural;
end package;

package body sizes is
  function double (arg : natural) return natural is
  begin
    return 2 * arg;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.sizes.all;

entity sized is
  generic (
    LANES : positive := 2;
    WIDTH : positive := double(LANES);
    INIT  : std_logic_vector(WIDTH - 1 downto 0) := (others => '1')
  );
  port (
    d : in  std_logic_vector(WIDTH - 1 downto 0);
    q : out std_logic_vector(WIDTH - 1 downto 0)
  );
end entity;

architecture rtl of sized is
begin
  q <= d xor INIT;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity wrapper is
  port (
    d : in  std_logic_vector(3 downto 0);
    q : out std_logic_vector(3 downto 0)
  );
end entity;

architecture rtl of wrapper is
begin
  -- Neither WIDTH nor INIT is associated.
  u : entity work.sized
    generic map (LANES => 2)
    port map (d => d, q => q);
end architecture;
