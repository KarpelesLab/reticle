-- Mixing up `<=` and `:=`, and declaring objects in the wrong place.
entity err_sigvar is
  port (
    d : in  bit;
    q : out bit
  );
end entity;

architecture rtl of err_sigvar is
  signal s : bit;
  variable bad_here : integer := 0;
begin
  process (d)
    variable v : bit;
    signal bad_signal : bit;
  begin
    v := d;
    s := v;
    v <= d;
    q <= v;
  end process;
end architecture;
