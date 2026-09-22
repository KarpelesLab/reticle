-- A procedure with a `signal` out parameter drives the caller's signal
-- directly; a `variable` out parameter is copied back.
entity procedure_call is
  port (
    d : in  bit_vector(3 downto 0);
    q : out bit_vector(3 downto 0);
    n : out integer
  );
end entity;

architecture rtl of procedure_call is
  procedure invert (signal o : out bit_vector(3 downto 0); v : in bit_vector(3 downto 0)) is
  begin
    o <= not v;
  end procedure;

  procedure count_ones (v : in bit_vector(3 downto 0); total : out integer) is
    variable c : integer := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        c := c + 1;
      end if;
    end loop;
    total := c;
  end procedure;
begin
  process (d)
    variable ones : integer;
  begin
    invert(q, d);
    count_ones(d, ones);
    n <= ones;
  end process;
end architecture;
