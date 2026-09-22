entity checker is
  port (clk, req, ack : in bit);
end entity;

architecture psl_arch of checker is
begin
  -- PSL directives are skipped with a warning; ordinary assertions stay.
  default clock is rising_edge(clk);
  assert always (req -> next ack);
  assume never (req and ack);
  cover (req and ack);
  assert req = '0' or ack = '1' report "plain assertion";
  restrict (req);
end architecture psl_arch;
