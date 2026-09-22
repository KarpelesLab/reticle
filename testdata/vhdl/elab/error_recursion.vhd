-- V0705: a recursive function cannot be inlined.
entity error_recursion is
  port (
    n : in  integer range 0 to 7;
    q : out integer
  );
end entity;

architecture rtl of error_recursion is
  function total (v : integer) return integer is
  begin
    if v <= 0 then
      return 0;
    end if;
    return v + total(v - 1);
  end function;
begin
  q <= total(n);
end architecture;
