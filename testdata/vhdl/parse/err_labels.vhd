entity e is
end entity f;

architecture rtl of e is
begin
  p : process
  begin
    l1 : loop
      exit;
    end loop l2;
    if x then
      null;
    end if oops;
  end process q;

  for i in 0 to 3 generate
    y(i) <= x(i);
  end generate;

  b : block
  begin
  end block c;
end architecture rtl;
