library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity gen_unit is
  generic (
    WIDTH  : positive := 8;
    INVERT : boolean := false;
    INIT   : std_logic_vector(7 downto 0) := x"0F";
    STEP   : integer := 1
  );
  port (
    a   : in  std_logic_vector(WIDTH - 1 downto 0);
    y   : out std_logic_vector(WIDTH - 1 downto 0);
    w   : out integer;
    iv  : out std_logic_vector(7 downto 0)
  );
end entity;

architecture rtl of gen_unit is
begin
  g_inv : if INVERT generate
    y <= std_logic_vector(unsigned(not a) + STEP);
  else generate
    y <= std_logic_vector(unsigned(a) + STEP);
  end generate;
  w <= WIDTH;
  iv <= INIT;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal a8, y8 : std_logic_vector(7 downto 0);
  signal a4, y4 : std_logic_vector(3 downto 0);
  signal w1, w2 : integer;
  signal i1, i2 : std_logic_vector(7 downto 0);
begin
  u1 : entity work.gen_unit
    port map (a => a8, y => y8, w => w1, iv => i1);
  u2 : entity work.gen_unit
    generic map (WIDTH => 4, INVERT => true, INIT => x"AB", STEP => 2)
    port map (a => a4, y => y4, w => w2, iv => i2);
  process
  begin
    a8 <= x"41";
    a4 <= x"3";
    wait for 1 ns;
    if y8 = x"42" then report "default generic ok"; else report "default generic bad"; end if;
    if y4 = x"E" then report "override generic ok"; else report "override generic bad"; end if;
    if w1 = 8 and w2 = 4 then report "width generic ok"; else report "width generic bad"; end if;
    if i1 = x"0F" then report "slv generic default ok"; else report "slv generic default bad"; end if;
    if i2 = x"AB" then report "slv generic override ok"; else report "slv generic override bad"; end if;
    wait;
  end process;
end architecture;
