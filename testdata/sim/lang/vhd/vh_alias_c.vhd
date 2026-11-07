library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  signal instr : std_logic_vector(31 downto 0) := x"12345678";
  alias opcode : std_logic_vector(7 downto 0) is instr(31 downto 24);
  alias rd : std_logic_vector(3 downto 0) is instr(23 downto 20);
  alias imm : std_logic_vector(15 downto 0) is instr(15 downto 0);
  alias low_nib is instr(3 downto 0);
  alias msb : std_logic is instr(31);
  alias rev : std_logic_vector(0 to 7) is instr(7 downto 0);
begin
  process
    variable v : unsigned(15 downto 0) := x"BEEF";
    alias vhi : unsigned(7 downto 0) is v(15 downto 8);
  begin
    if opcode = x"12" then report "alias slice ok"; else report "alias slice bad"; end if;
    if rd = x"3" then report "alias field ok"; else report "alias field bad"; end if;
    if imm = x"5678" then report "alias imm ok"; else report "alias imm bad"; end if;
    if low_nib = x"8" then report "untyped alias ok"; else report "untyped alias bad"; end if;
    if msb = '0' then report "scalar alias ok"; else report "scalar alias bad"; end if;
    if rev(0) = '0' and rev(1) = '1' and rev(7) = '0' then report "reversed alias ok"; else report "reversed alias bad"; end if;
    if std_logic_vector(vhi) = x"BE" then report "variable alias ok"; else report "variable alias bad"; end if;
    vhi := x"CA";
    if std_logic_vector(v) = x"CAEF" then report "write through var alias ok"; else report "write through var alias bad"; end if;
    opcode <= x"FF";
    wait for 1 ns;
    if instr = x"FF345678" then report "write through sig alias ok"; else report "write through sig alias bad"; end if;
    rev <= x"01";
    wait for 1 ns;
    if instr(7 downto 0) = x"80" then report "write reversed alias ok"; else report "write reversed alias bad"; end if;
    wait;
  end process;
end architecture;
