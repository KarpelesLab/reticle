library ieee;
use ieee.std_logic_1164.all;

entity assigns is
  port (a, b, c, clk : in std_logic; sel : in std_logic_vector(1 downto 0);
        y, z : out std_logic; v : out std_logic_vector(3 downto 0));
end entity;

architecture rtl of assigns is
  signal g : boolean;
  signal t : std_logic;
begin
  y <= a;
  y <= a after 1 ns;
  y <= '0', '1' after 5 ns, '0' after 10 ns;
  y <= transport a after 2 ns;
  y <= reject 1 ns inertial b;
  y <= inertial c;
  z <= unaffected;
  z <= null after 1 ns;

  cond : y <= a when sel = "00" else b when sel = "01" else c;
  cond_noelse : y <= a when sel = "00";
  cond_un : y <= a when g else unaffected;

  with sel select
    z <= a when "00",
         b when "01" | "10",
         c after 1 ns when others;

  gb : block (clk = '1')
  begin
    t <= guarded a;
    t <= guarded transport b after 3 ns;
    with sel select t <= guarded a when "00", b when others;
  end block gb;

  (v(0), v(1), v(2), v(3)) <= std_logic_vector'(a, b, c, a);
  v(1 downto 0) <= (others => '1');
  v(3 downto 2) <= b & c;

  post : postponed y <= a and b;
  postponed z <= not a;
  pc : postponed report_it(a);
  call_it(a, b);
  work.pkg.log("hello");
end architecture rtl;
