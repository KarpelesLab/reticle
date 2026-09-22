-- V0704: an unconstrained port has no width after elaboration.
entity error_unconstrained is
  port (
    d : in  bit_vector;
    q : out bit
  );
end entity;

architecture rtl of error_unconstrained is
begin
  q <= d(0);
end architecture;
