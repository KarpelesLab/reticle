-- Port map direction errors.
entity sub_block is
  port (
    a : in  bit;
    y : out bit;
    z : inout bit
  );
end entity;

architecture rtl of sub_block is
begin
  y <= a;
end architecture;

entity err_port_direction is
  port (
    pi : in  bit;
    po : out bit
  );
end entity;

architecture rtl of err_port_direction is
  signal s : bit;
begin
  u1 : entity work.sub_block
    port map (a => pi, y => pi, z => s);

  u2 : entity work.sub_block
    port map (a => s, y => po, z => pi);
end architecture;
