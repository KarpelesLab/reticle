-- `after` on a concurrent assignment becomes a transport delay; the
-- module's time unit is the femtosecond, VHDL's own resolution limit.
entity delays is
  port (
    a : in  bit;
    y : out bit;
    z : out bit
  );
end entity;

architecture rtl of delays is
  constant STEP : time := 2500 ps;
begin
  y <= not a after 5 ns;
  z <= a after STEP;
end architecture;
