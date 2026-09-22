-- Re-declaring a name in one region, and two units of the same name.
entity err_dup is
  port (
    clk : in  bit;
    clk : in  bit;
    q   : out bit
  );
end entity;

architecture rtl of err_dup is
  signal s : bit;
  signal S : integer;
  type t_t is (a, b);
  type t_t is range 0 to 3;
begin
  q <= clk;
end architecture;
