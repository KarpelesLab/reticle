library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  function fv(a, b : std_logic_vector(7 downto 0)) return std_logic_vector is
    variable v : std_logic_vector(7 downto 0);
  begin
    v := a xor b;
    return v;
  end function;
  function fn(a : std_logic_vector(7 downto 0)) return std_logic_vector is
  begin
    return not a;
  end function;
  function fb(a, b : std_logic) return std_logic is
  begin
    return a xor b;
  end function;
  signal s1 : std_logic_vector(7 downto 0) := x"0F";
begin
  process
    variable u : std_logic_vector(7 downto 0);
  begin
    u := fv(x"0F", x"05");
    if u = x"0A" then report "fv ok"; else report "fv bad"; end if;
    u := fn(x"0F");
    if u = x"F0" then report "fn ok"; else report "fn bad"; end if;
    if fb('1', '0') = '1' then report "fb ok"; else report "fb bad"; end if;
    u := fv(s1, x"05");
    if u = x"0A" then report "fv sig ok"; else report "fv sig bad"; end if;
    wait;
  end process;
end architecture;
