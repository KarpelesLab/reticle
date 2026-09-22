-- Return statements that do not match their subprogram.
package err_ret_pkg is
  function f (a : integer) return integer;
  procedure p (a : in integer);
end package;

package body err_ret_pkg is
  function f (a : integer) return integer is
  begin
    if a > 0 then
      return;
    end if;
    return "text";
  end function f;

  procedure p (a : in integer) is
  begin
    return a;
  end procedure p;
end package body;
