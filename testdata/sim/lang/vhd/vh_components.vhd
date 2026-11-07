library ieee;
use ieee.std_logic_1164.all;

entity half_add is
  port (a, b : in std_logic; s, c : out std_logic);
end entity;

architecture rtl of half_add is
begin
  s <= a xor b;
  c <= a and b;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity full_add is
  port (a, b, ci : in std_logic; s, co : out std_logic);
end entity;

architecture structural of full_add is
  component half_add is
    port (a, b : in std_logic; s, c : out std_logic);
  end component;
  signal s1, c1, c2 : std_logic;
begin
  h1 : half_add port map (a, b, s1, c1);
  h2 : half_add port map (a => s1, b => ci, s => s, c => c2);
  co <= c1 or c2;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  component full_add
    port (a, b, ci : in std_logic; s, co : out std_logic);
  end component;
  signal a, b : std_logic_vector(3 downto 0);
  signal s : std_logic_vector(3 downto 0);
  signal c : std_logic_vector(4 downto 0);
  signal open_s : std_logic;
  signal hs : std_logic;
begin
  c(0) <= '0';
  g : for i in 0 to 3 generate
    fa : full_add port map (a => a(i), b => b(i), ci => c(i), s => s(i), co => c(i + 1));
  end generate;
  -- direct instantiation, an output left open, an input tied to a constant
  h : entity work.half_add port map (a => '1', b => a(0), s => hs, c => open);
  process
  begin
    a <= "0101"; b <= "0011";
    wait for 1 ns;
    if s = "1000" and c(4) = '0' then report "add 5+3 ok"; else report "add 5+3 bad"; end if;
    if hs = '0' then report "direct inst ok"; else report "direct inst bad"; end if;
    a <= "1111"; b <= "0001";
    wait for 1 ns;
    if s = "0000" and c(4) = '1' then report "add carry ok"; else report "add carry bad"; end if;
    if hs = '0' then report "direct inst 2 ok"; else report "direct inst 2 bad"; end if;
    a <= "1010"; b <= "0100";
    wait for 1 ns;
    if s = "1110" and c(4) = '0' then report "add 10+4 ok"; else report "add 10+4 bad"; end if;
    if hs = '1' then report "direct inst 3 ok"; else report "direct inst 3 bad"; end if;
    wait;
  end process;
end architecture;
