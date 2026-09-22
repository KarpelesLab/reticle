-- Conditional and selected concurrent signal assignments.
entity concurrent is
  port (
    a, b, c : in  bit;
    sel     : in  bit_vector(1 downto 0);
    y       : out bit;
    z       : out bit;
    w       : out bit
  );
end entity;

architecture rtl of concurrent is
  signal t : bit;
begin
  y <= a when sel = "00" else
       b when sel = "01" else
       c;

  with sel select
    z <= a    when "00",
         b    when "01" | "10",
         '0'  when others;

  t <= a and b;
  w <= t after 5 ns;
end architecture;
