-- The elements of a record are reached by selection only (clause 12.3), so
-- resolving `r.size` must not make `size` a name in the enclosing region.
-- It did, and then a signal of the same name resolved to the element, a
-- parameter of the same name was a redeclaration, and the design was
-- analysed against the wrong types.
--
-- Found by the Colibri corpus; see docs/vhdl-corpus.md.
library ieee;
use ieee.std_logic_1164.all;

package rec_pkg is

  type state_t is record
    size : natural;
    done : std_logic;
  end record;

end package rec_pkg;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rec_pkg.all;

entity rec_user is
  port (
    clk_i : in  std_logic;
    q_o   : out std_logic
  );
end entity;

architecture rtl of rec_user is

  signal reg  : state_t;
  -- Same name as an element of `state_t`, and a different type.
  signal size : std_logic_vector(3 downto 0) := "0001";

  -- A parameter named after an element, and one taking the element's own
  -- subtype through `'subtype`.
  function pick (done : std_logic; size : reg.size'subtype) return std_logic is
  begin
    if size > 0 then
      return done;
    end if;
    return '0';
  end function;

begin

  process (clk_i) is
  begin
    if rising_edge(clk_i) then
      -- The selections that used to leak `size` and `done` into the region.
      reg.size <= to_integer(unsigned(size));
      reg.done <= size(0);
    end if;
  end process;

  q_o <= pick(reg.done, reg.size);

end architecture;
