-- The predefined operators on integers and bit vectors, with the explicit
-- `resize` nodes the IR's equal-width rule demands.
entity operators is
  port (
    a, b : in  integer range 0 to 255;
    v    : in  bit_vector(7 downto 0);
    w    : in  bit_vector(3 downto 0);
    sum  : out integer;
    r_rem : out integer;
    r_mod : out integer;
    cat  : out bit_vector(11 downto 0);
    red  : out bit;
    cmp  : out boolean
  );
end entity;

architecture rtl of operators is
begin
  sum  <= a + b * 2 - 1;
  r_rem <= a rem 3;
  r_mod <= (a - 200) mod 7;
  cat  <= v & w;
  red  <= xor v;
  cmp  <= a < b and v /= "00000000";
end architecture;
