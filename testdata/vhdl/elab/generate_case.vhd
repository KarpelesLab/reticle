-- A VHDL-2008 case-generate; the chosen alternative's label becomes part
-- of the hierarchical prefix.
entity generate_case is
  generic (MODE : natural := 1);
  port (
    a, b : in  bit;
    q    : out bit
  );
end entity;

architecture rtl of generate_case is
begin
  pick : case MODE generate
    when zero : 0 =>
      q <= a and b;
    when one : 1 =>
      q <= a or b;
    when rest : others =>
      q <= a xor b;
  end generate;
end architecture;
