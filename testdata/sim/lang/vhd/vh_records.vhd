library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package rec_pkg is
  type point_t is record
    x : integer;
    y : integer;
  end record;
  type pkt_t is record
    valid : std_logic;
    addr  : unsigned(7 downto 0);
    data  : std_logic_vector(15 downto 0);
    pos   : point_t;
  end record;
  type pkt_arr is array (natural range <>) of pkt_t;
  constant PKT_ZERO : pkt_t := (valid => '0', addr => (others => '0'), data => (others => '0'), pos => (0, 0));
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rec_pkg.all;

entity rec_unit is
  port (i : in pkt_t; o : out pkt_t);
end entity;

architecture rtl of rec_unit is
begin
  process (i)
    variable t : pkt_t;
  begin
    t := i;
    t.addr := i.addr + 1;
    t.pos.x := i.pos.y;
    t.pos.y := i.pos.x;
    o <= t;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rec_pkg.all;

entity tb is
end entity;

architecture sim of tb is
  signal pin, pout : pkt_t := PKT_ZERO;
  signal arr : pkt_arr(0 to 2) := (others => PKT_ZERO);
begin
  dut : entity work.rec_unit port map (i => pin, o => pout);
  process
    variable p : point_t := (3, 4);
    variable k : pkt_t;
  begin
    if p.x = 3 and p.y = 4 then report "positional rec ok"; else report "positional rec bad"; end if;
    p := (y => 9, x => 8);
    if p.x = 8 and p.y = 9 then report "named rec ok"; else report "named rec bad"; end if;
    k := (valid => '1', addr => x"10", data => x"BEEF", pos => (1, 2));
    if k.data = x"BEEF" and k.pos.y = 2 then report "nested agg ok"; else report "nested agg bad"; end if;
    if k /= PKT_ZERO then report "rec neq ok"; else report "rec neq bad"; end if;
    pin <= k;
    wait for 1 ns;
    if pout.addr = x"11" then report "port addr ok"; else report "port addr bad"; end if;
    if pout.pos.x = 2 and pout.pos.y = 1 then report "port nested ok"; else report "port nested bad"; end if;
    if pout.valid = '1' and pout.data = x"BEEF" then report "port pass ok"; else report "port pass bad"; end if;
    pin.addr <= x"FF";
    wait for 1 ns;
    if pout.addr = x"00" then report "field sig assign ok"; else report "field sig assign bad"; end if;
    arr(1) <= k;
    arr(2).data <= x"1234";
    wait for 1 ns;
    if arr(1).addr = x"10" and arr(0).valid = '0' then report "rec array ok"; else report "rec array bad"; end if;
    if arr(2).data = x"1234" and arr(2).valid = '0' then report "rec array field ok"; else report "rec array field bad"; end if;
    if arr(1) = k then report "rec eq ok"; else report "rec eq bad"; end if;
    report "pout.pos.x=" & integer'image(pout.pos.x); -- img
    wait;
  end process;
end architecture;
