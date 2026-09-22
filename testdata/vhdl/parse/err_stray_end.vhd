architecture rtl of e is
begin
  p : process (clk)
  begin
    y <= x;
    end loop;
    z <= y;
  end process p;
  end if;
  q <= z;
end architecture rtl;
