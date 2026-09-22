-- Type mismatches in assignments, initialisers and conditions.
entity err_type_mismatch is
  port (
    b : in  bit;
    n : in  integer;
    q : out bit
  );
end entity;

architecture rtl of err_type_mismatch is
  signal flag  : boolean;
  signal count : integer;
  signal v     : bit_vector(3 downto 0);
  constant BAD : integer := "hello";
begin
  count <= b;
  flag  <= n;
  q     <= count;
  v     <= b;
end architecture;
