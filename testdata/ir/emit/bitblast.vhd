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

entity inv is
  port (
    a : in std_logic;
    y : out std_logic
  );
end entity inv;

architecture rtl of inv is
begin
  y <= (not a);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.reticle_pkg.all;

entity gates is
  port (
    clk : in std_logic;
    rst : in std_logic;
    en : in std_logic;
    a : in unsigned(1 downto 0);
    b : in unsigned(1 downto 0);
    s : in std_logic;
    sel : in unsigned(2 downto 0);
    n : out unsigned(1 downto 0);
    o : out unsigned(1 downto 0);
    x : out unsigned(1 downto 0);
    m : out unsigned(1 downto 0);
    p : out unsigned(1 downto 0);
    \all\ : out std_logic;
    any : out std_logic;
    par : out std_logic;
    f : out std_logic;
    q : out unsigned(1 downto 0);
    r : out unsigned(1 downto 0);
    l : out unsigned(1 downto 0);
    copy : out unsigned(1 downto 0);
    inv_out : out std_logic;
    bb_out : out std_logic
  );
  attribute init : integer;
  attribute init of r : signal is 1;
end entity gates;

architecture rtl of gates is
  constant lut0_init : unsigned(7 downto 0) := "10010110";
  component SB_LUT4 is
    generic (
      LUT_INIT : integer := 32768
    );
    port (
      I0 : in std_logic;
      I1 : in std_logic;
      I2 : in std_logic;
      I3 : in std_logic;
      O : out std_logic
    );
  end component;
begin
  copy <= unsigned'(a(0) & '1');
  n <= (not a);
  o <= (a or b);
  x <= (a xor unsigned'("11"));
  m <= b when s = '1' else a;
  p <= pmux(a, unsigned'(b & a & n), sel);
  \all\ <= (and a);
  any <= (or a);
  par <= (xor unsigned'(a & b));
  f <= lut0_init(to_integer(unsigned'(s & a)));
  ff0 : process (clk)
  begin
    if rising_edge(clk) then
      q <= a;
    end if;
  end process;
  ff1 : process (clk)
  begin
    if falling_edge(clk) then
      if rst = '0' then
        r <= unsigned'("10");
      else
        if en = '1' then r <= b; end if;
      end if;
    end if;
  end process;
  lat0 : process (all)
  begin
    if en = '1' then l <= m; end if;
  end process;
  bb0 : SB_LUT4 generic map (LUT_INIT => 32768) port map (
    I0 => s,
    I1 => \all\,
    I2 => '0',
    I3 => 'X',
    O => bb_out
  );
  u_inv : entity work.inv port map (
    a => s,
    y => inv_out
  );
end architecture rtl;
