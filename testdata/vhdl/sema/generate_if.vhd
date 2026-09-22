-- An if-generate selected by a static generic.
entity gen_if is
  generic (INVERT : boolean := true);
  port (
    d : in  bit;
    q : out bit
  );
end entity;

architecture rtl of gen_if is
begin
  inv : if INVERT generate
    q <= not d;
  else generate
    q <= d;
  end generate inv;
end architecture;
