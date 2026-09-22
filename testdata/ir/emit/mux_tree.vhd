library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package reticle_pkg is
  function to_sl(v : unsigned) return std_logic;
  function to_sl(b : boolean) return std_logic;
  function idx(v : unsigned; i : integer) return std_logic;
  function slc(v : unsigned; hi, lo : integer) return unsigned;
  function rep(v : unsigned; n : natural) return unsigned;
  function mux(s : std_logic; t, f : std_logic) return std_logic;
  function mux(s : std_logic; t, f : unsigned) return unsigned;
  function mux(s : std_logic; t, f : signed) return signed;
  function mux(s : std_logic; t, f : integer) return integer;
  function mux(s : std_logic; t, f : real) return real;
  function pmux(a, b, s : unsigned) return unsigned;
  function pow(a, b : unsigned) return unsigned;
  function pow(a, b : signed) return signed;
end package reticle_pkg;

package body reticle_pkg is
  function to_sl(v : unsigned) return std_logic is
    variable n : unsigned(v'length - 1 downto 0) := v;
  begin
    return n(0);
  end function;
  function to_sl(b : boolean) return std_logic is
  begin
    if b then return '1'; else return '0'; end if;
  end function;
  function idx(v : unsigned; i : integer) return std_logic is
    variable n : unsigned(v'length - 1 downto 0) := v;
  begin
    if i < 0 or i >= n'length then return 'X'; end if;
    return n(i);
  end function;
  function slc(v : unsigned; hi, lo : integer) return unsigned is
    variable n : unsigned(v'length - 1 downto 0) := v;
    variable r : unsigned(hi - lo downto 0) := (others => 'X');
  begin
    for k in 0 to hi - lo loop
      if lo + k >= 0 and lo + k < n'length then r(k) := n(lo + k); end if;
    end loop;
    return r;
  end function;
  function rep(v : unsigned; n : natural) return unsigned is
    variable r : unsigned(v'length * n - 1 downto 0);
  begin
    for k in 0 to n - 1 loop
      r((k + 1) * v'length - 1 downto k * v'length) := v;
    end loop;
    return r;
  end function;
  function mux(s : std_logic; t, f : std_logic) return std_logic is
  begin
    if s = '1' then return t; else return f; end if;
  end function;
  function mux(s : std_logic; t, f : unsigned) return unsigned is
  begin
    if s = '1' then return t; else return f; end if;
  end function;
  function mux(s : std_logic; t, f : signed) return signed is
  begin
    if s = '1' then return t; else return f; end if;
  end function;
  function mux(s : std_logic; t, f : integer) return integer is
  begin
    if s = '1' then return t; else return f; end if;
  end function;
  function mux(s : std_logic; t, f : real) return real is
  begin
    if s = '1' then return t; else return f; end if;
  end function;
  function pmux(a, b, s : unsigned) return unsigned is
    variable r : unsigned(a'length - 1 downto 0) := a;
    variable bn : unsigned(b'length - 1 downto 0) := b;
    variable sn : unsigned(s'length - 1 downto 0) := s;
  begin
    for k in 0 to sn'length - 1 loop
      if sn(k) = '1' then r := bn((k + 1) * r'length - 1 downto k * r'length); end if;
    end loop;
    return r;
  end function;
  function pow(a, b : unsigned) return unsigned is
    variable r : unsigned(a'length - 1 downto 0) := (others => '0');
    variable base : unsigned(a'length - 1 downto 0) := a;
    variable e : unsigned(b'length - 1 downto 0) := b;
  begin
    r(0) := '1';
    for k in 0 to e'length - 1 loop
      if e(k) = '1' then r := resize(r * base, r'length); end if;
      base := resize(base * base, base'length);
    end loop;
    return r;
  end function;
  function pow(a, b : signed) return signed is
  begin
    return signed(pow(unsigned(a), unsigned(b)));
  end function;
end package body reticle_pkg;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.reticle_pkg.all;

entity mux4 is
  port (
    sel : in unsigned(1 downto 0);
    a : in unsigned(7 downto 0);
    b : in unsigned(7 downto 0);
    c : in unsigned(7 downto 0);
    d : in unsigned(7 downto 0);
    y : out unsigned(7 downto 0);
    y2 : out unsigned(7 downto 0)
  );
end entity mux4;

architecture rtl of mux4 is
  signal lo : unsigned(7 downto 0);
  signal hi : unsigned(7 downto 0);
begin
  lo <= mux(sel(0), b, a);
  hi <= mux(sel(1), d, c);
  y <= mux(sel(1), hi, lo);
  decode : process (all)
    variable y2_v : unsigned(7 downto 0);
  begin
    y2_v := y2;
    case sel is
      when "00" =>
        y2_v := a;
      when "01" =>
        y2_v := b;
      when "10" | "11" =>
        y2_v := mux(sel(0), d, c);
      when others =>
        null;
    end case;
    y2 <= y2_v;
  end process;
end architecture rtl;
