-- `wait`, signal attributes and edges outside a clock condition, initial
-- values without an event, and `std.env.finish`, held to what Vivado
-- 2026.1's xsim printed for this file (waits_events_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;

entity tb is end entity;

architecture sim of tb is
  signal s : integer := 0;
  signal clk : std_logic := '1';
  signal other : integer := 0;
  signal a : std_logic := '0';
  signal b : std_logic := '0';
  signal n : integer := 7;
  signal dly : std_logic;
  signal c0 : std_logic := '0';
  signal c1 : std_logic := '1';
  signal nf, nr : integer := 0;
begin
  dly <= b'delayed(2 ns);
  pf : process (c0) begin if falling_edge(c0) then nf <= nf + 1; end if; end process;
  pr : process (c1) begin if rising_edge(c1) then nr <= nr + 1; end if; end process;

  process
  begin
    report "start";
    -- `wait for 0 ns` is the next delta: the update is visible.
    s <= 5;
    wait for 0 ns;
    report "after a delta s=" & integer'image(s);
    -- `wait until` waits for an event even when the condition holds.
    other <= 1 after 3 ns;
    clk <= '0' after 10 ns, '1' after 20 ns;
    wait until clk = '1';
    report "resumed at " & time'image(now) & " other=" & integer'image(other)
      & " edges " & integer'image(nf) & integer'image(nr);
    -- A timeout.
    wait until a = '1' for 7 ns;
    report "timed out at " & time'image(now);
    a <= '1' after 2 ns;
    wait until a = '1' for 7 ns;
    report "condition at " & time'image(now);
    wait on a for 4 ns;
    report "wait on timed out at " & time'image(now);
    -- Edges and signal attributes as values.
    if rising_edge(b) then report "edge at once"; else report "no edge at once"; end if;
    b <= '1';
    n <= 9;
    wait on b;
    if rising_edge(b) then report "rising edge"; end if;
    if b'event and b'last_value = '0' and n'last_value = 7 then report "event, last values"; end if;
    if b'active then report "active"; end if;
    wait for 0 ns;
    if b'event then report "event outlived its delta"; else report "event over a delta later"; end if;
    wait for 1 ns;
    if not b'event and b'stable then report "no event, stable"; end if;
    if b'last_event = 1 ns then report "last event 1 ns ago"; end if;
    if dly = '0' then report "delayed still 0"; end if;
    if not b'stable(3 ns) then report "not stable for 3 ns"; end if;
    wait for 2 ns;
    if dly = '1' then report "delayed now 1"; end if;
    b <= '0' after 1 ns;
    wait until falling_edge(b);
    report "falling edge at " & time'image(now);
    clk <= '0' after 1 ns, '1' after 2 ns;
    wait until rising_edge(clk);
    report "clock rose at " & time'image(now);
    std.env.finish;
    report "not reached";
    wait;
  end process;
end architecture;
