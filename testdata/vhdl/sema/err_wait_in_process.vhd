-- `wait` in a process with a sensitivity list, and in a function.
package err_wait_pkg is
  function f (a : integer) return integer;
end package;

package body err_wait_pkg is
  function f (a : integer) return integer is
  begin
    wait for 10 ns;
    return a;
  end function f;
end package body;

entity err_wait is
  port (
    clk : in  bit;
    q   : out bit
  );
end entity;

architecture rtl of err_wait is
begin
  p : process (clk)
  begin
    wait until clk = '1';
    q <= clk;
  end process;
end architecture;
