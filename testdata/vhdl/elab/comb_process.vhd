-- A combinational process with an explicit sensitivity list next to one
-- with the VHDL-2008 `process (all)`.
entity comb_process is
  port (
    a, b, c : in  bit;
    y       : out bit;
    z       : out bit
  );
end entity;

architecture rtl of comb_process is
  signal t : bit;
begin
  explicit : process (a, b)
  begin
    t <= a and b;
  end process;

  all_list : process (all)
  begin
    y <= t xor c;
    z <= not t;
  end process;
end architecture;
