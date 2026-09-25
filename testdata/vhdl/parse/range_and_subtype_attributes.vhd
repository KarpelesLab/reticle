-- The attributes that are spelled with a reserved word, and the two places
-- where a range attribute stands where an expression could: as the choice of
-- an aggregate and as the discrete range of a slice. A value attribute in
-- the same positions must stay an expression.
--
-- Found by the Colibri corpus; see docs/vhdl-corpus.md.
library ieee;
use ieee.std_logic_1164.all;

entity attrs is
  port (
    data_i : in  std_logic_vector(7 downto 0);
    sel_i  : in  integer;
    data_o : out std_logic_vector(7 downto 0)
  );
end entity;

architecture rtl of attrs is

  -- `'subtype`, a reserved word as an attribute designator.
  signal copy : data_i'subtype;
  signal zero : data_i'subtype := (data_i'range => '0');
  signal ones : std_logic_vector(3 downto 0) := (others => '1');

begin

  copy                 <= data_i;
  data_o(data_i'range) <= (data_i'range => '0') when sel_i = 0 else data_i;

  process (sel_i) is
  begin
    case sel_i is
      -- A value attribute is still an expression here.
      when integer'high => null;
      when 0 to 3       => null;
      when others       => null;
    end case;
  end process;

end architecture;
