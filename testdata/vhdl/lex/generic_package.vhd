-- VHDL-2008: a package with generics, a generic-mapped instance, a context
-- declaration, matching relational operators and external names.
context proj_ctx is
  library ieee;
  use ieee.std_logic_1164.all, ieee.numeric_std.all;
end context proj_ctx;

package fifo_pkg is
  generic (
    type data_t;
    DEPTH : positive := 16;
    function eq(a, b : data_t) return boolean is <>
  );
  type mem_t is array (0 to DEPTH - 1) of data_t;
  function is_full(cnt : natural) return boolean;
end package fifo_pkg;

package byte_fifo_pkg is new work.fifo_pkg
  generic map (data_t => std_logic_vector(7 downto 0), DEPTH => 32);

architecture rtl of top is
  alias probe is << signal .top.dut.state : std_logic_vector >>;
  signal ok : boolean;
begin
  ok <= a ?= b and not (c ?/= d) and (e ?< f) and (g ?>= h);
  q <= ?? valid;
  case? sel is
    when "1-" => y <= '1';
    when others => y <= '0';
  end case?;
  probe <= force "0000";
  probe <= release;
  <<signal ^.^.sibling : bit>> <= '1';
  x <= @lib.pkg.cst;
  assert always (req -> next ack);
end architecture;
