library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  type distance is range 0 to 100000000
    units
      mm;
      cm = 10 mm;
      m = 100 cm;
      km = 1000 m;
    end units;
  constant period : time := 10 ns;
  signal t_sig : time := 0 ns;
begin
  process
    variable t : time;
    variable d : distance;
    variable n : integer;
  begin
    report "now0=" & time'image(now);
    wait for 5 ns;
    report "now1=" & time'image(now);
    t := period * 3;
    report "period*3=" & time'image(t);
    t := period / 4;
    report "period/4=" & time'image(t);
    report "period/1ns=" & integer'image(period / 1 ns);
    report "1us=" & time'image(1 us);
    report "1500ps=" & time'image(1500 ps);
    report "1.5 ns=" & time'image(1.5 ns);
    report "2*ns=" & time'image(2 * 1 ns);
    report "ms+us=" & time'image(1 ms + 1 us);
    report "neg=" & time'image(-period);
    report "abs=" & time'image(abs (-period));
    report "compare " & boolean'image(1 us > 999 ns);
    report "time'pos(1ns)=" & integer'image(time'pos(1 ns));
    d := 2 km + 5 cm;
    report "d=" & distance'image(d);
    report "d/mm=" & integer'image(d / 1 mm);
    report "d in m=" & integer'image(d / 1 m);
    report "1 m=" & distance'image(1 m);
    wait for period;
    report "now2=" & time'image(now);
    t_sig <= now after 3 ns;
    wait for 4 ns;
    report "t_sig=" & time'image(t_sig);
    n := now / 1 ps;
    report "now in ps=" & integer'image(n);
    report "to_string(now)=" & to_string(now);
    report "to_string(2 ns, ps)=" & to_string(2 ns, ps);
    wait;
  end process;
end architecture;
