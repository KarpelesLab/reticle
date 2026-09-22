-- A for-generate replicating a bit-level assignment.
entity gen_for is
  port (
    d : in  bit_vector(7 downto 0);
    q : out bit_vector(7 downto 0)
  );
end entity;

architecture rtl of gen_for is
  signal t : bit_vector(7 downto 0);
begin
  bits : for i in 0 to 7 generate
    t(i) <= not d(i);
  end generate bits;

  rev : for i in d'range generate
    q(7 - i) <= t(i);
  end generate;
end architecture;
