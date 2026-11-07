library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity counter is
  generic (WIDTH : positive := 4; MAXV : natural := 11);
  port (
    clk, rst, en : in std_logic;
    q    : out unsigned(WIDTH - 1 downto 0);
    wrap : out std_logic
  );
end entity;

architecture rtl of counter is
  signal cnt : unsigned(WIDTH - 1 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      wrap <= '0';
      if rst = '1' then
        cnt <= (others => '0');
      elsif en = '1' then
        if cnt = MAXV then
          cnt <= (others => '0');
          wrap <= '1';
        else
          cnt <= cnt + 1;
        end if;
      end if;
    end if;
  end process;
  q <= cnt;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  signal clk : std_logic := '0';
  signal rst, en : std_logic := '0';
  signal q : unsigned(3 downto 0);
  signal wrap : std_logic;
  signal stop : boolean := false;
  signal errors : integer := 0;
  signal wraps : integer := 0;
begin
  clk <= not clk after 5 ns when not stop else clk;
  dut : entity work.counter generic map (WIDTH => 4, MAXV => 11)
    port map (clk => clk, rst => rst, en => en, q => q, wrap => wrap);

  wcount : process (clk)
  begin
    if rising_edge(clk) then
      if wrap = '1' then
        wraps <= wraps + 1;
      end if;
    end if;
  end process;

  process
    variable expect : integer := 0;
    variable errs : integer := 0;
  begin
    rst <= '1';
    wait until clk = '1';
    wait until clk = '0';
    rst <= '0';
    en <= '1';
    for i in 1 to 30 loop
      wait until clk = '1';
      wait until clk = '0';
      expect := (expect + 1) mod 12;
      if to_integer(q) /= expect then
        errs := errs + 1;
      end if;
    end loop;
    if errs = 0 then report "count sequence ok"; else report "count sequence bad"; end if;
    en <= '0';
    wait until clk = '1';
    wait until clk = '0';
    if to_integer(q) = expect then report "hold ok"; else report "hold bad"; end if;
    if wraps = 2 then report "wrap count ok"; else report "wrap count bad"; end if;
    rst <= '1';
    wait until clk = '1';
    wait until clk = '0';
    if q = 0 then report "reset ok"; else report "reset bad"; end if;
    stop <= true;
    wait;
  end process;
end architecture;
