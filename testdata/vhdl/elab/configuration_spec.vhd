-- top: cfg_top
-- A configuration specification overrides the default binding, choosing
-- the entity and the architecture for each instance by label.
entity cell_impl is
  port (
    d : in  bit;
    q : out bit
  );
end entity;

architecture buffered of cell_impl is
begin
  q <= d;
end architecture;

architecture inverted of cell_impl is
begin
  q <= not d;
end architecture;

entity cfg_top is
  port (
    din : in  bit;
    x   : out bit;
    y   : out bit
  );
end entity;

architecture rtl of cfg_top is
  component cell is
    port (
      d : in  bit;
      q : out bit
    );
  end component;

  for u_buf : cell use entity work.cell_impl(buffered);
  for u_inv : cell use entity work.cell_impl(inverted);
begin
  u_buf : component cell port map (d => din, q => x);
  u_inv : component cell port map (d => din, q => y);
end architecture;
