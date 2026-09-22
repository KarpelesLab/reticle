-- Writing to things that cannot be written.
entity err_assign_constant is
  generic (WIDTH : positive := 8);
  port (
    d : in  bit;
    q : out bit
  );
end entity;

architecture rtl of err_assign_constant is
  constant LIMIT : natural := 10;
begin
  process (d)
  begin
    LIMIT <= 5;
    WIDTH <= 4;
    d     <= '1';
    for i in 0 to 3 loop
      i := 2;
    end loop;
  end process;

  q <= d;
end architecture;
