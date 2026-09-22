library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;

entity tb is
end entity tb;

architecture sim of tb is
  signal clk : std_logic := '0';
  signal done : boolean := false;
  signal v : std_logic_vector(7 downto 0);
begin
  clk <= not clk after 5 ns when not done else unaffected;

  stim : process
    variable i, j : integer := 0;
    variable l : line;
    variable arr : bit_vector(0 to 7);
  begin
    wait;
    wait for 10 ns;
    wait on clk;
    wait on clk, done;
    wait until clk = '1';
    wait until rising_edge(clk) for 100 ns;
    wait on clk until clk = '1' for 1 us;

    i := 0;
    i := i + 1;
    v <= (others => '0');
    v(0) <= '1';
    (arr(0), arr(1)) := bit_vector'("10");

    outer : for k in 0 to 7 loop
      inner : for m in v'range loop
        next inner when m = k;
        exit outer when k = 5;
        next;
        exit;
      end loop inner;
    end loop outer;

    for n in natural range 1 to 3 loop
      null;
    end loop;

    for e in some_enum loop
      i := i + 1;
    end loop;

    while i < 10 loop
      i := i + 1;
      if i = 5 then
        next;
      end if;
    end loop;

    forever : loop
      exit forever when i > 20;
      i := i + 1;
    end loop forever;

    if i = 0 then
      j := 1;
    elsif i = 1 then
      j := 2;
    elsif i = 2 then
      j := 3;
    else
      j := 4;
    end if;

    if i = 0 then
      null;
    end if;

    lbl : if j > 0 then
      j := 0;
    end if lbl;

    case i is
      when 0 => j := 1;
      when 1 | 2 => j := 2;
      when 3 to 5 => j := 3;
      when others => null;
    end case;

    sel : case v(1 downto 0) is
      when "00" =>
        j := 0;
        i := 0;
      when others =>
    end case sel;

    report "done" severity note;
    report "value is " & integer'image(i);
    assert i /= 0;
    assert i /= 0 report "zero";
    assert i /= 0 report "zero" severity warning;
    write(l, string'("hello"));
    writeline(output, l);
    std.env.finish;
    done <= true;
    wait;
  end process stim;

  fn : process
    function inc(x : integer) return integer is
    begin
      return x + 1;
    end function inc;
    procedure p is
    begin
      return;
    end procedure p;
  begin
    p;
    wait;
  end process fn;
end architecture sim;
