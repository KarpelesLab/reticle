-- Unknown formals, missing connections and duplicate associations.
entity leaf is
  generic (WIDTH : positive);
  port (
    clk : in  bit;
    d   : in  bit;
    q   : out bit
  );
end entity;

architecture rtl of leaf is
begin
  q <= d;
end architecture;

entity err_port_map is
  port (
    clk : in  bit;
    q   : out bit
  );
end entity;

architecture rtl of err_port_map is
  signal s : bit;
begin
  u1 : entity work.leaf
    generic map (WIDTH => 8)
    port map (clk => clk, data => s, q => q);

  u2 : entity work.leaf
    generic map (WIDTH => 8)
    port map (clk => clk, q => q);

  u3 : entity work.leaf
    port map (clk => clk, d => s, q => q);
end architecture;
