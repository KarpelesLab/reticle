-- A function that drives a signal, and a pure function calling an impure one.
package err_fn_pkg is
  signal shared_flag : bit;

  function bad (a : integer) return integer;
  impure function clock_count return integer;
  function also_bad (a : integer) return integer;
end package err_fn_pkg;

package body err_fn_pkg is
  function bad (a : integer) return integer is
  begin
    shared_flag <= '1';
    return a;
  end function bad;

  impure function clock_count return integer is
  begin
    return 0;
  end function clock_count;

  function also_bad (a : integer) return integer is
  begin
    return a + clock_count;
  end function also_bad;
end package body err_fn_pkg;
