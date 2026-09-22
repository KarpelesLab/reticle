-- A selected concurrent assignment becomes a chain of equality tests: the
-- expression form of a `case`.
entity selected_assign is
  port (
    a, b : in  bit;
    sel  : in  bit_vector(1 downto 0);
    z    : out bit
  );
end entity;

architecture rtl of selected_assign is
begin
  with sel select
    z <= a   when "00",
         b   when "01" | "10",
         '0' when others;
end architecture;
