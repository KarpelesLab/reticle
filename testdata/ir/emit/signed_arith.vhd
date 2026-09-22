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

entity alu is
  port (
    a : in signed(7 downto 0);
    b : in signed(7 downto 0);
    u : in unsigned(7 downto 0);
    n : in unsigned(2 downto 0);
    sum : out signed(7 downto 0);
    diff : out signed(7 downto 0);
    prod : out signed(7 downto 0);
    quot : out signed(7 downto 0);
    \rem\ : out signed(7 downto 0);
    pw : out signed(7 downto 0);
    asr : out signed(7 downto 0);
    usr : out unsigned(7 downto 0);
    lsr : out signed(7 downto 0);
    shl : out signed(7 downto 0);
    lt : out std_logic;
    ge : out std_logic;
    eq : out std_logic;
    mixed : out unsigned(7 downto 0);
    ext : out signed(15 downto 0);
    zext : out unsigned(15 downto 0);
    trunc : out unsigned(3 downto 0);
    sext_sum : out signed(15 downto 0);
    trunc_sum : out unsigned(3 downto 0);
    neg : out signed(7 downto 0);
    sel : out signed(7 downto 0);
    cat : out unsigned(15 downto 0);
    \rep\ : out unsigned(15 downto 0);
    \bit\ : out std_logic;
    part : out unsigned(3 downto 0);
    part_dn : out unsigned(3 downto 0);
    par : out std_logic;
    wild : out std_logic;
    exact : out std_logic;
    lnot : out std_logic;
    land : out std_logic;
    nested : out signed(7 downto 0);
    clk : in std_logic;
    uq : out unsigned(7 downto 0)
  );
end entity alu;

architecture rtl of alu is
  signal reticle_ff_clk : std_logic;
begin
  sum <= (a + b);
  diff <= (a - b);
  prod <= signed(resize(unsigned(a) * unsigned(b), 8));
  quot <= (a / b);
  \rem\ <= (a rem b);
  pw <= pow(a, signed'("00000010"));
  asr <= shift_right(a, to_integer(n));
  usr <= unsigned(shift_right(signed(u), to_integer(n)));
  lsr <= signed(shift_right(unsigned(a), to_integer(n)));
  shl <= shift_left(a, to_integer(unsigned'("00000001")));
  lt <= (a ?< b);
  ge <= (a ?>= signed'("00000000"));
  eq <= (u ?= unsigned(a));
  mixed <= (u + unsigned(a));
  ext <= resize(a, 16);
  zext <= resize(unsigned(a), 16);
  trunc <= resize(unsigned(a), 4);
  sext_sum <= resize((a + b), 16);
  trunc_sum <= resize((u + u), 4);
  neg <= (-a);
  sel <= mux(lt, a, b);
  cat <= unsigned'(unsigned(a) & u);
  \rep\ <= rep(u, 2);
  \bit\ <= a(to_integer(n));
  part <= u(to_integer(n) + 3 downto to_integer(n));
  part_dn <= slc(unsigned((a + b)), to_integer(unsigned'("00000111")), to_integer(unsigned'("00000111")) - 3);
  par <= (xor u);
  wild <= to_sl(std_match(u, unsigned'("1---0--1")));
  exact <= to_sl(u = unsigned'("XXXXXXXX"));
  lnot <= (nor u);
  land <= (lt and ge);
  nested <= (a - (b + signed(resize(unsigned(a) * unsigned(signed'("00000011")), 8))));
  reticle_ff_clk <= (not clk);
  ff : process (reticle_ff_clk)
  begin
    if rising_edge(reticle_ff_clk) then
      uq <= u;
    end if;
  end process;
end architecture rtl;
