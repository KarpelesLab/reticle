library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
begin
  process
    variable s, i, j, c : integer;
  begin
    s := 0;
    for k in 1 to 10 loop
      s := s + k;
    end loop;
    if s = 55 then report "for ok"; else report "for bad"; end if;
    s := 0;
    for k in 10 downto 1 loop
      s := s * 2 + (k mod 2);
    end loop;
    if s = 682 then report "downto ok"; else report "downto bad"; end if;
    s := 0;
    for k in 5 to 1 loop
      s := s + 1;
    end loop;
    if s = 0 then report "null range ok"; else report "null range bad"; end if;
    i := 0;
    while i < 7 loop
      i := i + 2;
    end loop;
    if i = 8 then report "while ok"; else report "while bad"; end if;
    i := 0;
    loop
      i := i + 1;
      exit when i = 4;
    end loop;
    if i = 4 then report "plain loop exit ok"; else report "plain loop exit bad"; end if;
    s := 0;
    for k in 1 to 10 loop
      next when k mod 3 = 0;
      s := s + k;
    end loop;
    if s = 37 then report "next when ok"; else report "next when bad"; end if;
    c := 0;
    j := 0;
    l1 : while true loop
      j := j + 1;
      if j > 5 then
        exit l1;
      end if;
      if j = 2 then
        next l1;
      end if;
      c := c + j;
    end loop l1;
    if c = 13 then report "while true exit ok"; else report "while true exit bad"; end if;
    s := 0;
    for k in boolean loop
      if k then s := s + 1; end if;
    end loop;
    if s = 1 then report "enum loop ok"; else report "enum loop bad"; end if;
    wait;
  end process;
end architecture;
