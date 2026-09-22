architecture rtl of e is
begin
  p1 : process (clk)
  begin
    if rising_edge(clk) then
      for i in 0 to 3 loop
        x(i) <= y(i);
    end if;
  end process p1;

  p2 : process (clk)
  begin
    case s is
      when '0' => y <= '1';
      when others => y <= '0';
  end process p2;

  y <= z;
end architecture rtl;

entity next_unit is
end entity;
