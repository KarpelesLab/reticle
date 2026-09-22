-- V0702: a component with no entity of the same name in `work` and no
-- configuration specification binds to nothing.
entity error_unbound is
  port (
    d : in  bit;
    q : out bit
  );
end entity;

architecture rtl of error_unbound is
  component missing_cell is
    port (
      a : in  bit;
      y : out bit
    );
  end component;
begin
  u0 : component missing_cell port map (a => d, y => q);
end architecture;
