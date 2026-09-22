-- top: wrapper
-- Direct entity instantiation, with and without an architecture name.
entity leaf is
  port (
    d : in  bit;
    q : out bit
  );
end entity;

architecture inverting of leaf is
begin
  q <= not d;
end architecture;

architecture passing of leaf is
begin
  q <= d;
end architecture;

entity wrapper is
  port (
    d : in  bit;
    a : out bit;
    b : out bit
  );
end entity;

architecture rtl of wrapper is
begin
  u_default : entity work.leaf
    port map (d => d, q => a);

  u_inverting : entity work.leaf(inverting)
    port map (d => d, q => b);
end architecture;
