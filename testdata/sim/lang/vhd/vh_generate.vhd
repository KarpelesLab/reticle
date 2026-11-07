library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  constant N : integer := 4;
  constant MODE : integer := 2;
  type sel_t is (A_SEL, B_SEL, C_SEL);
  constant SEL : sel_t := B_SEL;
  signal x : std_logic_vector(N - 1 downto 0) := "1011";
  signal y : std_logic_vector(N - 1 downto 0);
  signal chain : std_logic_vector(N downto 0);
  signal m : integer;
  signal c : integer;
  type int_arr is array (0 to N - 1) of integer;
  signal sq : int_arr;
begin
  g_for : for i in 0 to N - 1 generate
    y(i) <= x(N - 1 - i);
  end generate;

  chain(0) <= '1';
  g_chain : for i in 1 to N generate
    signal t : std_logic;
  begin
    t <= chain(i - 1) and x(i - 1);
    chain(i) <= t;
  end generate;

  g_sq : for i in sq'range generate
    sq(i) <= i * i;
  end generate;

  g_if : if MODE = 1 generate
    m <= 10;
  elsif MODE = 2 generate
    m <= 20;
  else generate
    m <= 30;
  end generate;

  g_case : case SEL generate
    when A_SEL => c <= 1;
    when B_SEL => c <= 2;
    when others => c <= 3;
  end generate;

  g_nest : for i in 0 to 1 generate
    g_inner : if i = 1 generate
      -- nothing but a nested generate
    end generate;
  end generate;

  process
  begin
    wait for 1 ns;
    if y = "1101" then report "for generate ok"; else report "for generate bad"; end if;
    if chain = "00011" then report "chain generate ok"; else report "chain generate bad"; end if;
    if sq(3) = 9 and sq(2) = 4 then report "range generate ok"; else report "range generate bad"; end if;
    if m = 20 then report "elsif generate ok"; else report "elsif generate bad"; end if;
    if c = 2 then report "case generate ok"; else report "case generate bad"; end if;
    x <= "1111";
    wait for 1 ns;
    if chain = "11111" then report "chain update ok"; else report "chain update bad"; end if;
    wait;
  end process;
end architecture;
