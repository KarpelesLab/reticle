library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity ram is
  port (
    clk : in std_logic;
    we : in std_logic;
    addr : in unsigned(3 downto 0);
    wdata : in unsigned(7 downto 0);
    rdata : out unsigned(7 downto 0);
    rom_out : out unsigned(7 downto 0)
  );
end entity ram;

architecture rtl of ram is
  type mem_t is array (0 to 15) of unsigned(7 downto 0);
  type rom_t is array (0 to 3) of unsigned(7 downto 0);
  attribute ram_style : string;
  signal mem : mem_t;
  signal rom : rom_t := ("11011110", "10101101", "10111110", "11101111");
  attribute ram_style of mem : signal is "block";
begin
  rom_out <= rom(to_integer(addr(1 downto 0)));
  process (clk)
  begin
    if rising_edge(clk) then
      if we = '1' then mem(to_integer(addr)) <= wdata; end if;
      rdata <= mem(to_integer(addr));
    end if;
  end process;
end architecture rtl;
