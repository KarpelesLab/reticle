-- V0706: two drivers of an unresolved signal cannot be resolved.
entity error_drivers is
  port (
    a, b : in  bit;
    y    : out bit
  );
end entity;

architecture rtl of error_drivers is
  signal net_s : bit;
begin
  net_s <= a;
  net_s <= b;

  y <= net_s;
end architecture;
