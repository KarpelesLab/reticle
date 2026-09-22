-- VHDL-2008 features.
library ieee;
use ieee.std_logic_1164.all;

package gen_pkg is
  generic (
    type data_t;
    DEPTH : positive := 16;
    function eq(a, b : data_t) return boolean is <>;
    package inner is new work.other_pkg generic map (<>);
    package dflt is new work.other_pkg generic map (default)
  );
  type mem_t is array (0 to DEPTH - 1) of data_t;
  function is_full(cnt : natural) return boolean;
end package gen_pkg;

package byte_pkg is new work.gen_pkg
  generic map (data_t => std_logic_vector(7 downto 0), DEPTH => 32);

entity top is
  port (
    clk, a, b, c, d : in std_logic;
    sel  : in  std_logic_vector(1 downto 0);
    y, q : out std_logic
  );
end entity;

architecture rtl of top is
  subtype rvec is (resolved) std_logic_vector;
  subtype rrec is (a resolved, b (resolved)) some_rec_t;
  signal ok      : boolean;
  signal state   : std_logic_vector(3 downto 0);
  signal v       : std_logic_vector(7 downto 0);
  alias probe is << signal .top.dut.state : std_logic_vector >>;
  alias cst is << constant @lib.pkg.cst : integer >>;
  alias up is << variable ^.^.sibling.v : bit >>;
  shared variable counter : integer := 0;
begin
  ok <= a ?= b and not (c ?/= d) and (e ?< f) and (g ?>= h);
  q <= ?? valid;
  y <= and v;
  y <= or v and nand w;

  comb : process (all)
    variable t : std_logic;
  begin
    t := a when sel = "00" else b when sel = "01" else c;
    with sel select t :=
      a when "00",
      b when "01",
      '0' when others;
    y <= t;
    case? sel is
      when "1-" => y <= '1';
      when "0-" => y <= '0';
      when others => y <= 'X';
    end case?;
    with sel select? y <=
      a when "1-",
      b when others;
  end process;

  probe <= force "0000";
  probe <= force in "1111" when a = '1' else "0000";
  probe <= release;
  probe <= release out;
  <<signal ^.^.sibling : bit>> <= '1';
  x <= << constant @lib.pkg.cst : integer >>;

  u : entity work.sub
    port map (clk => clk, d => inertial v, q => open);

  g : if a = '1' generate
    y <= '1';
  else generate
    y <= '0';
  end generate;

  proc : process is
    variable p : integer;
  begin
    p := 1 when a = '1' else 0;
    wait;
  end process;
end architecture rtl;
