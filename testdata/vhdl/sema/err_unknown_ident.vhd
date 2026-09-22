-- Unknown identifiers, with "did you mean" over the visible names.
entity err_unknown is
  port (
    clock : in  bit;
    reset : in  bit;
    q     : out bit
  );
end entity;

architecture rtl of err_unknown is
  signal counter : natural := 0;
begin
  q <= clok;

  process (clock)
  begin
    if reset = '1' then
      countr <= 0;
    end if;
  end process;
end architecture;
