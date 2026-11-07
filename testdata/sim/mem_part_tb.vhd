-- Writes to part of an array element in VHDL: `mem(a)(15 downto 8) <= d`
-- and `mem(a)(i) <= '1'` on a signal in a clocked process, the byte writes
-- one `if` per lane as one process must write them (a process per lane
-- would each drive the whole of `mem`, since `mem(addr)` is no static name);
-- an element of an ascending vector type; and the same on a variable, a
-- constant index and one known only at run time. It checks each value
-- against what xsim 2026.1 printed for it (`mem_part_tb_vhd.xsim` beside it
-- has those lines, from the same writes with a `report` of each value) and
-- reports `FAIL` for any that differs, `PASS` at the end.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity mem_part_tb is
end entity;

architecture sim of mem_part_tb is
  type mem_t is array (0 to 3) of std_logic_vector(31 downto 0);
  type asc_t is array (0 to 1) of std_logic_vector(0 to 15);
  signal mem : mem_t := (others => (others => '0'));
  signal asc : asc_t := (others => (others => '1'));
  signal clk : std_logic := '0';
  signal phase : natural := 0;
  signal we : std_logic_vector(3 downto 0) := "0000";
  signal addr : natural range 0 to 3 := 0;
  signal bit_at : natural range 0 to 31 := 0;
  signal d : std_logic_vector(31 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if phase = 1 then
        mem(0)(7 downto 0) <= x"11";
        mem(0)(31 downto 24) <= x"44";
        mem(1)(16) <= '1';
        mem(1)(bit_at) <= '1';
        asc(0)(0 to 3) <= "0000";
        asc(1)(12 to 15) <= x"5";
        asc(1)(1) <= '0';
      elsif phase = 2 then
        if we(0) = '1' then
          mem(addr)(7 downto 0) <= d(7 downto 0);
        end if;
        if we(1) = '1' then
          mem(addr)(15 downto 8) <= d(15 downto 8);
        end if;
        if we(2) = '1' then
          mem(addr)(23 downto 16) <= d(23 downto 16);
        end if;
        if we(3) = '1' then
          mem(addr)(31 downto 24) <= d(31 downto 24);
        end if;
      end if;
    end if;
  end process;

  process
    variable v : mem_t := (others => (others => '0'));
    variable k : natural;
    variable bad : natural := 0;
  begin
    if mem(0) /= x"00000000" then
      report "FAIL: mem(0)" severity error;
      bad := bad + 1;
    end if;
    if asc(0) /= x"FFFF" then
      report "FAIL: asc(0)" severity error;
      bad := bad + 1;
    end if;
    phase <= 1;
    bit_at <= 3;
    wait for 1 ns;
    clk <= '1';
    wait for 1 ns;
    clk <= '0';
    -- "direct: 44000011 00010008 00000000 | 0FFF BFF5"
    if mem(0) /= x"44000011" then
      report "FAIL: mem(0)" severity error;
      bad := bad + 1;
    end if;
    if mem(1) /= x"00010008" then
      report "FAIL: mem(1)" severity error;
      bad := bad + 1;
    end if;
    if mem(2) /= x"00000000" then
      report "FAIL: mem(2)" severity error;
      bad := bad + 1;
    end if;
    if asc(0) /= x"0FFF" then
      report "FAIL: asc(0)" severity error;
      bad := bad + 1;
    end if;
    if asc(1) /= x"BFF5" then
      report "FAIL: asc(1)" severity error;
      bad := bad + 1;
    end if;
    phase <= 2;
    addr <= 2;
    d <= x"deadbeef";
    we <= "0101";
    wait for 1 ns;
    clk <= '1';
    wait for 1 ns;
    clk <= '0';
    -- "lanes: 00AD00EF"
    if mem(2) /= x"00AD00EF" then
      report "FAIL: mem(2)" severity error;
      bad := bad + 1;
    end if;
    we <= "1010";
    d <= x"01234567";
    wait for 1 ns;
    clk <= '1';
    wait for 1 ns;
    clk <= '0';
    -- "lanes2: 01AD45EF"
    if mem(2) /= x"01AD45EF" then
      report "FAIL: mem(2)" severity error;
      bad := bad + 1;
    end if;
    v(1)(15 downto 8) := x"aa";
    v(1)(0) := '1';
    k := 30;
    v(2)(k) := '1';
    v(2)(7 downto 4) := x"c";
    -- "variable: 0000AA01 400000C0"
    if v(1) /= x"0000AA01" then
      report "FAIL: v(1)" severity error;
      bad := bad + 1;
    end if;
    if v(2) /= x"400000C0" then
      report "FAIL: v(2)" severity error;
      bad := bad + 1;
    end if;
    if bad = 0 then
      report "PASS" severity note;
    end if;
    wait;
  end process;
end architecture;
