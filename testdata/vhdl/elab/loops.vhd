-- `for` becomes an IR `for` with a variable loop parameter, so `exit` and
-- `next` stay expressible; `while` and the bare `loop` map directly.
entity loops is
  port (
    d : in  bit_vector(7 downto 0);
    n : out integer;
    m : out integer
  );
end entity;

architecture rtl of loops is
begin
  process (d)
    variable count : integer;
    variable i     : integer;
  begin
    count := 0;
    for a in d'range loop
      next when d(a) = '0';
      count := count + 1;
    end loop;
    n <= count;

    i := 0;
    while i < 8 loop
      exit when d(i) = '1';
      i := i + 1;
    end loop;
    m <= i;
  end process;
end architecture;
