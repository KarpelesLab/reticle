-- Loop forms with labelled exit and next.
entity loops is
  port (
    d : in  bit_vector(7 downto 0);
    q : out natural
  );
end entity;

architecture rtl of loops is
begin
  process (d)
    variable count : natural;
    variable i     : natural;
  begin
    count := 0;

    outer : for a in d'range loop
      inner : for b in 0 to 3 loop
        next inner when d(a) = '0';
        exit outer when count > 100;
        count := count + 1;
      end loop inner;
    end loop outer;

    i := 0;
    while i < 8 loop
      count := count + 1;
      i := i + 1;
    end loop;

    endless : loop
      exit endless;
    end loop;

    q <= count;
  end process;
end architecture;
