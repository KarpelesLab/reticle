architecture rtl of e is
begin
  y <= a and b or c;
  z <= a nand b nand c;
  w <= a xor b and c or d;
  ok <= (a and b) or c;
  p <= 2 ** 3 ** 4;
end architecture rtl;
