-- An object alias binds to the part of the net the aliased name denotes.
entity aliases is
  port (
    d   : in  bit_vector(15 downto 0);
    hi  : out bit_vector(7 downto 0);
    top : out bit
  );
end entity;

architecture rtl of aliases is
  alias upper   : bit_vector(7 downto 0) is d(15 downto 8);
  alias top_bit : bit is d(15);
begin
  hi  <= upper;
  top <= top_bit;
end architecture;
