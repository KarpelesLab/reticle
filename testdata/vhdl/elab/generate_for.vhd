-- A for-generate is unrolled into the enclosing module, every copy under
-- the hierarchical prefix `bits(n).`.
entity generate_for is
  generic (N : positive := 4);
  port (
    d : in  bit_vector(N - 1 downto 0);
    q : out bit_vector(N - 1 downto 0)
  );
end entity;

architecture rtl of generate_for is
  signal t : bit_vector(N - 1 downto 0);
begin
  bits : for i in 0 to N - 1 generate
    signal inv : bit;
  begin
    inv  <= not d(i);
    t(i) <= inv;
  end generate;

  q <= t;
end architecture;
