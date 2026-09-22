-- top: nowhere
-- V0700: the requested top entity is not in the design.
entity only_entity is
  port (
    d : in  bit;
    q : out bit
  );
end entity;

architecture rtl of only_entity is
begin
  q <= d;
end architecture;
