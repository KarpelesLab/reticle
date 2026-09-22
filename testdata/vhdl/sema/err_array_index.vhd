-- Indexing and slicing outside an object's static range.
entity err_index is
  port (
    d : in  bit_vector(7 downto 0);
    q : out bit
  );
end entity;

architecture rtl of err_index is
  signal s : bit_vector(3 downto 0);
  signal n : natural;
begin
  q <= d(8);
  s <= d(9 downto 6);
  s <= d(0 to 3);
  q <= n(0);
end architecture;
