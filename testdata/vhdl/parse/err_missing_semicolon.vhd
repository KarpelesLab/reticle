entity e is
  port (a : in bit; y : out bit);
end entity e;

architecture rtl of e is
  signal x : bit
  signal z : bit;
  constant K : integer := 1
begin
  x <= a
  z <= x;
  y <= z;
  process (a)
  begin
    if a = '1' then
      x <= '0'
    end if;
  end process;
end architecture rtl;
