library ieee;
use ieee.std_logic_1164.all;
entity sub is
  generic (W : positive := 8);
  port (a : in std_logic_vector(W downto 0);
        sv, ss : out std_logic_vector(W - 1 downto 0));
end entity;
architecture rtl of sub is
begin
  ss <= a(W - 1 downto 0);
  process (a)
    variable v : std_logic_vector(8 downto 0);
  begin
    v := a;
    sv <= v(W - 1 downto 0);
  end process;
end architecture;
library ieee;
use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  constant K : integer := 8;
  signal a : std_logic_vector(8 downto 0) := "010000000";
  signal sv, ss : std_logic_vector(7 downto 0);
begin
  u : entity work.sub port map (a, sv, ss);
  process
    variable v : std_logic_vector(8 downto 0) := "010000000";
  begin
    wait for 1 ns;
    if ss = x"80" then report "signal slice by generic ok"; else report "signal slice by generic bad"; end if;
    if sv = x"80" then report "variable slice by generic ok"; else report "variable slice by generic bad"; end if;
    if v(K - 1 downto 0) = x"80" then report "variable slice by constant ok"; else report "variable slice by constant bad"; end if;
    wait;
  end process;
end architecture;
