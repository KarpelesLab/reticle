-- V0701: the IR's `break` leaves the innermost loop only, so an `exit`
-- naming an outer loop is named as unsupported.
entity error_exit_label is
  port (
    d : in  bit_vector(3 downto 0);
    q : out integer
  );
end entity;

architecture rtl of error_exit_label is
begin
  process (d)
    variable count : integer;
  begin
    count := 0;
    outer : for a in d'range loop
      inner : for b in 0 to 1 loop
        exit outer when d(a) = '1';
        count := count + 1;
      end loop;
    end loop;
    q <= count;
  end process;
end architecture;
