library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity gadd is
  generic (W : positive := 8);
  port (
    clk  : in std_logic;
    a, b : in std_logic_vector(W - 1 downto 0);
    cin  : in std_logic;
    s    : out std_logic_vector(W - 1 downto 0);
    cout : out std_logic;
    ovf  : out std_logic
  );
end entity;

architecture rtl of gadd is
begin
  process (clk)
    variable full : unsigned(W downto 0);
    variable c : unsigned(0 downto 0);
  begin
    if rising_edge(clk) then
      c(0) := cin;
      full := ('0' & unsigned(a)) + ('0' & unsigned(b)) + c;
      s <= std_logic_vector(full(W - 1 downto 0));
      cout <= full(W);
      ovf <= (a(W - 1) xnor b(W - 1)) and (a(W - 1) xor full(W - 1));
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  signal clk : std_logic := '0';
  signal stop : boolean := false;
  signal a8, b8, s8 : std_logic_vector(7 downto 0) := (others => '0');
  signal a12, b12, s12 : std_logic_vector(11 downto 0) := (others => '0');
  signal cin, c8, c12, o8, o12 : std_logic := '0';
begin
  clk <= not clk after 5 ns when not stop else clk;
  u8 : entity work.gadd generic map (W => 8)
    port map (clk => clk, a => a8, b => b8, cin => cin, s => s8, cout => c8, ovf => o8);
  u12 : entity work.gadd generic map (W => 12)
    port map (clk => clk, a => a12, b => b12, cin => cin, s => s12, cout => c12, ovf => o12);
  process
    variable errs : integer := 0;
    variable x, y, e : integer;
  begin
    for i in 0 to 15 loop
      x := (i * 37 + 11) mod 256;
      y := (i * 91 + 5) mod 256;
      a8 <= std_logic_vector(to_unsigned(x, 8));
      b8 <= std_logic_vector(to_unsigned(y, 8));
      a12 <= std_logic_vector(to_unsigned(x * 13, 12));
      b12 <= std_logic_vector(to_unsigned(y * 15, 12));
      if i mod 3 = 0 then cin <= '1'; else cin <= '0'; end if;
      wait until clk = '1';
      wait until clk = '0';
      e := x + y + (1 - (i mod 3 + 2) / 3);
      if i mod 3 = 0 then e := x + y + 1; else e := x + y; end if;
      if to_integer(unsigned(s8)) /= e mod 256 then errs := errs + 1; end if;
      if (c8 = '1') /= (e >= 256) then errs := errs + 1; end if;
      if i mod 3 = 0 then e := x * 13 + y * 15 + 1; else e := x * 13 + y * 15; end if;
      if to_integer(unsigned(s12)) /= e mod 4096 then errs := errs + 1; end if;
      if (c12 = '1') /= (e >= 4096) then errs := errs + 1; end if;
    end loop;
    if errs = 0 then report "adder results ok"; else report "adder results bad"; end if;
    a8 <= x"7F"; b8 <= x"01"; cin <= '0';
    wait until clk = '1';
    wait until clk = '0';
    if o8 = '1' and s8 = x"80" then report "overflow ok"; else report "overflow bad"; end if;
    a8 <= x"FF"; b8 <= x"01";
    wait until clk = '1';
    wait until clk = '0';
    if o8 = '0' and c8 = '1' and s8 = x"00" then report "carry no overflow ok"; else report "carry no overflow bad"; end if;
    stop <= true;
    wait;
  end process;
end architecture;
