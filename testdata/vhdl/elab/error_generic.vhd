-- top: error_generic
-- V0703: a generic with no default and no value in the generic map.
entity error_generic is
  generic (WIDTH : positive);
  port (
    d : in  bit_vector(7 downto 0);
    q : out bit_vector(7 downto 0)
  );
end entity;

architecture rtl of error_generic is
begin
  q <= d;
end architecture;
