-- Enumeration literals are encoded by declaration order and the mapping
-- is recorded on every net of the type.
entity enum_encoding is
  port (
    sel : in  bit_vector(2 downto 0);
    ok  : out boolean
  );
end entity;

architecture rtl of enum_encoding is
  type colour_t is (red, orange, yellow, green, blue, indigo, violet);
  signal colour : colour_t := green;
begin
  process (sel)
  begin
    case sel is
      when "000"  => colour <= red;
      when "001"  => colour <= orange;
      when "010"  => colour <= yellow;
      when others => colour <= violet;
    end case;
  end process;

  ok <= colour = violet;
end architecture;
