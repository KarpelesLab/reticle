-- An if-generate chooses one arm from a static generic.
entity generate_if is
  generic (INVERT : boolean := true);
  port (
    d : in  bit;
    q : out bit
  );
end entity;

architecture rtl of generate_if is
begin
  choice : if INVERT generate
    q <= not d;
  else generate
    q <= d;
  end generate;
end architecture;
