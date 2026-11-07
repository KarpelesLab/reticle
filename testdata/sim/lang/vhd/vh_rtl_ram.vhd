library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- simple dual-port RAM, synchronous read (read-first)
entity ram is
  generic (AW : positive := 4; DW : positive := 8);
  port (
    clk   : in std_logic;
    we    : in std_logic;
    waddr : in unsigned(AW - 1 downto 0);
    wdata : in std_logic_vector(DW - 1 downto 0);
    raddr : in unsigned(AW - 1 downto 0);
    rdata : out std_logic_vector(DW - 1 downto 0)
  );
end entity;

architecture rtl of ram is
  type mem_t is array (0 to 2 ** AW - 1) of std_logic_vector(DW - 1 downto 0);
  signal mem : mem_t := (others => (others => '0'));
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if we = '1' then
        mem(to_integer(waddr)) <= wdata;
      end if;
      rdata <= mem(to_integer(raddr));
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
  signal we : std_logic := '0';
  signal waddr, raddr : unsigned(3 downto 0) := (others => '0');
  signal wdata, rdata : std_logic_vector(7 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns when not stop else clk;
  dut : entity work.ram generic map (AW => 4, DW => 8)
    port map (clk => clk, we => we, waddr => waddr, wdata => wdata, raddr => raddr, rdata => rdata);
  process
    variable errs : integer := 0;
  begin
    -- write a pattern
    for i in 0 to 15 loop
      we <= '1';
      waddr <= to_unsigned(i, 4);
      wdata <= std_logic_vector(to_unsigned((i * 17 + 3) mod 256, 8));
      wait until clk = '1';
      wait until clk = '0';
    end loop;
    we <= '0';
    -- read it back (one cycle latency)
    for i in 0 to 15 loop
      raddr <= to_unsigned(15 - i, 4);
      wait until clk = '1';
      wait until clk = '0';
      if to_integer(unsigned(rdata)) /= ((15 - i) * 17 + 3) mod 256 then
        errs := errs + 1;
      end if;
    end loop;
    if errs = 0 then report "ram readback ok"; else report "ram readback bad"; end if;
    -- read-first: write and read the same address in one cycle
    we <= '1';
    waddr <= to_unsigned(5, 4);
    raddr <= to_unsigned(5, 4);
    wdata <= x"EE";
    wait until clk = '1';
    wait until clk = '0';
    we <= '0';
    if to_integer(unsigned(rdata)) = (5 * 17 + 3) mod 256 then report "read-first old data ok"; else report "read-first old data bad"; end if;
    wait until clk = '1';
    wait until clk = '0';
    if rdata = x"EE" then report "new data next cycle ok"; else report "new data next cycle bad"; end if;
    stop <= true;
    wait;
  end process;
end architecture;
