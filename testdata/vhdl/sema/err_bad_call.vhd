-- Calls with the wrong number, names or types of arguments.
package call_pkg is
  function add3 (a, b, c : integer) return integer;
  procedure pulse (signal s : out bit; constant width : in time);
end package call_pkg;

package body call_pkg is
  function add3 (a, b, c : integer) return integer is
  begin
    return a + b + c;
  end function add3;

  procedure pulse (signal s : out bit; constant width : in time) is
  begin
    s <= '1';
  end procedure pulse;
end package body call_pkg;

use work.call_pkg.all;

entity err_bad_call is
  port (q : out integer);
end entity;

architecture rtl of err_bad_call is
  signal s : bit;
  constant K : integer := 1;
begin
  q <= add3(1, 2);
  q <= add3(1, 2, 3, 4);
  q <= add3(a => 1, bee => 2, c => 3);
  q <= add3(1, 2, true);

  process
  begin
    pulse(K, 10 ns);
    wait;
  end process;
end architecture;
