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

entity datapath is
  port (
    clk : in std_logic;
    rst_n : in std_logic;
    en : in std_logic;
    a : in signed(7 downto 0);
    b : in signed(7 downto 0);
    u : in unsigned(7 downto 0);
    v : in unsigned(7 downto 0);
    n : in unsigned(2 downto 0);
    sel : in unsigned(1 downto 0);
    sum : out signed(7 downto 0);
    diff : out unsigned(7 downto 0);
    prod : out signed(7 downto 0);
    quot : out signed(7 downto 0);
    \rem\ : out unsigned(7 downto 0);
    shl : out signed(7 downto 0);
    shr : out unsigned(7 downto 0);
    sshr : out signed(7 downto 0);
    eq : out std_logic;
    ne : out std_logic;
    lt : out std_logic;
    le : out std_logic;
    gt : out std_logic;
    ge : out std_logic;
    pm : out unsigned(7 downto 0);
    q : out signed(7 downto 0);
    q2 : out unsigned(7 downto 0);
    rd : out unsigned(7 downto 0);
    raddr : in unsigned(1 downto 0);
    waddr : in unsigned(1 downto 0)
  );
end entity datapath;

architecture rtl of datapath is
  type buf_t is array (0 to 3) of unsigned(7 downto 0);
  signal buf : buf_t;
begin
  sum <= (a + b);
  diff <= (u - v);
  prod <= signed(resize(unsigned(a) * unsigned(b), 8));
  quot <= (a / b);
  \rem\ <= (u rem v);
  shl <= shift_left(a, to_integer(n));
  shr <= shift_right(u, to_integer(n));
  sshr <= shift_right(a, to_integer(n));
  eq <= (a ?= b);
  ne <= (u ?/= v);
  lt <= (a ?< b);
  le <= (a ?<= b);
  gt <= (u ?> v);
  ge <= (u ?>= v);
  pm <= pmux(u, unsigned'(v & diff), sel);
  ff0 : process (clk, rst_n)
  begin
    if rst_n = '0' then
      q <= signed'("00000000");
    elsif rising_edge(clk) then
      if en = '1' then q <= sum; end if;
    end if;
  end process;
  ff1 : process (clk, en)
  begin
    if en = '1' then
      q2 <= unsigned'("11111111");
    elsif falling_edge(clk) then
      q2 <= diff;
    end if;
  end process;
  rd0 : process (clk)
  begin
    if rising_edge(clk) and en = '1' then rd <= buf(to_integer(raddr)); end if;
  end process;
  wr0 : process (all)
  begin
    if en = '1' then buf(to_integer(waddr)) <= u; end if;
  end process;
end architecture rtl;
