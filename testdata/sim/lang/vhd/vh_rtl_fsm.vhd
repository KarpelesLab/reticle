library ieee;
use ieee.std_logic_1164.all;

-- sequence detector for 1011 (overlapping), Moore output
entity seqdet is
  port (clk, rst, din : in std_logic; found : out std_logic);
end entity;

architecture rtl of seqdet is
  type state_t is (S0, S1, S10, S101, S1011);
  signal st, nx : state_t;
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        st <= S0;
      else
        st <= nx;
      end if;
    end if;
  end process;

  comb : process (all)
  begin
    nx <= st;
    case st is
      when S0 =>
        if din = '1' then nx <= S1; end if;
      when S1 =>
        if din = '0' then nx <= S10; end if;
      when S10 =>
        if din = '1' then nx <= S101; else nx <= S0; end if;
      when S101 =>
        if din = '1' then nx <= S1011; else nx <= S10; end if;
      when S1011 =>
        if din = '1' then nx <= S1; else nx <= S10; end if;
    end case;
  end process;

  found <= '1' when st = S1011 else '0';
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  constant PATTERN : std_logic_vector(0 to 15) := "1011011010110000";
  constant EXPECT  : std_logic_vector(0 to 15) := "0000100100001000";
  signal clk : std_logic := '0';
  signal rst : std_logic := '1';
  signal din, found : std_logic := '0';
  signal stop : boolean := false;
begin
  clk <= not clk after 5 ns when not stop else clk;
  dut : entity work.seqdet port map (clk => clk, rst => rst, din => din, found => found);
  process
    variable errs, hits : integer := 0;
  begin
    wait until clk = '1';
    wait until clk = '0';
    rst <= '0';
    for i in PATTERN'range loop
      din <= PATTERN(i);
      wait until clk = '1';
      wait until clk = '0';
      if found /= EXPECT(i) then
        errs := errs + 1;
      end if;
      if found = '1' then
        hits := hits + 1;
      end if;
    end loop;
    if errs = 0 then report "fsm output sequence ok"; else report "fsm output sequence bad"; end if;
    if hits = 3 then report "fsm hit count ok"; else report "fsm hit count bad"; end if;
    stop <= true;
    wait;
  end process;
end architecture;
