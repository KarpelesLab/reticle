-- A width computed by the design's own function, which elaboration has to
-- interpret: `log2ceil` is how portable VHDL sizes a counter, and it turns
-- up in a generic default (`WIDTH`), in a signal's bounds and in a slice.
-- `maximum` is the one `std.standard` declares for every scalar type
-- (LRM 5.2.6), whose operands here are not locally static.
--
-- Found by the Colibri corpus; see docs/vhdl-corpus.md.
library ieee;
use ieee.std_logic_1164.all;

package helpers is
  function log2ceil (arg : natural) return natural;
  function pattern (bits : natural) return std_logic_vector;
end package;

package body helpers is

  -- The same shape Colibri's `utils` package uses: a while loop over
  -- variables, with an early return.
  function log2ceil (arg : natural) return natural is
    variable step : positive := 1;
    variable bits : natural  := 0;
  begin
    if arg < 1 then
      return 0;
    end if;
    while arg > step loop
      step := step * 2;
      bits := bits + 1;
    end loop;
    return bits;
  end function;

  -- A for loop, a case and an indexed assignment, interpreted into a
  -- constant of the width the function above computed.
  function pattern (bits : natural) return std_logic_vector is
    variable out_v : std_logic_vector(bits - 1 downto 0);
  begin
    for i in out_v'range loop
      case i mod 3 is
        when 0      => out_v(i) := '1';
        when others => out_v(i) := '0';
      end case;
    end loop;
    return out_v;
  end function;

end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.helpers.all;

entity static_function is
  generic (
    DEPTH : positive := 100;
    WIDTH : natural  := log2ceil(DEPTH)
  );
  port (
    clk : in  std_logic;
    q   : out std_logic_vector(maximum(WIDTH, 4) - 1 downto 0)
  );
end entity;

architecture rtl of static_function is

  constant SEED : std_logic_vector(WIDTH - 1 downto 0) := pattern(WIDTH);
  signal reg    : std_logic_vector(maximum(WIDTH, 4) - 1 downto 0);

begin

  process (clk)
  begin
    if rising_edge(clk) then
      reg(SEED'range) <= reg(SEED'range) xor SEED;
    end if;
  end process;

  q <= reg;

end architecture;
