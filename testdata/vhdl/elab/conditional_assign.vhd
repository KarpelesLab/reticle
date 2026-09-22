-- A conditional concurrent assignment becomes a chain of `mux` nodes.
entity conditional_assign is
  port (
    a, b, c : in  bit;
    sel     : in  bit_vector(1 downto 0);
    y       : out bit
  );
end entity;

architecture rtl of conditional_assign is
begin
  y <= a when sel = "00" else
       b when sel = "01" else
       c;
end architecture;
