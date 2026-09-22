-- top: error_port_map
-- V0708: an `out` port must be connected to something that can be driven.
entity sink is
  port (
    d : in  bit;
    q : out bit
  );
end entity;

architecture rtl of sink is
begin
  q <= d;
end architecture;

entity error_port_map is
  port (
    a : in  bit;
    b : in  bit;
    y : out bit
  );
end entity;

architecture rtl of error_port_map is
begin
  u0 : entity work.sink port map (d => a, q => open);
  u1 : entity work.sink port map (d => a, q => (a and b));
  y <= a;
end architecture;
