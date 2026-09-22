entity e is
  generic (
    N : positive := 4;
    M positive;
    K : natural := 2
  );
  port (
    a : in bit;
    b : bit_vector(3 downto 0;
    c : out bit;
  );
end entity e;

architecture rtl of e is
begin
  c <= a;
end architecture rtl;
