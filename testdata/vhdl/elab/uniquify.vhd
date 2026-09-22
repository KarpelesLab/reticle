-- top: pair
-- One entity instantiated with two generic sets becomes two modules, the
-- second named after the generic that differs.
entity shifter is
  generic (WIDTH : positive := 4);
  port (
    d : in  bit_vector(WIDTH - 1 downto 0);
    q : out bit_vector(WIDTH - 1 downto 0)
  );
end entity;

architecture rtl of shifter is
begin
  q <= d;
end architecture;

entity pair is
  port (
    a : in  bit_vector(3 downto 0);
    b : in  bit_vector(7 downto 0);
    x : out bit_vector(3 downto 0);
    y : out bit_vector(7 downto 0)
  );
end entity;

architecture rtl of pair is
begin
  u4 : entity work.shifter port map (d => a, q => x);
  u8 : entity work.shifter generic map (WIDTH => 8) port map (d => b, q => y);
end architecture;
