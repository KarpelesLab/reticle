-- A block statement is flattened into the enclosing module under its
-- label.
entity block_stmt is
  port (
    a, b : in  bit;
    y    : out bit
  );
end entity;

architecture rtl of block_stmt is
begin
  inner : block
    signal t : bit;
  begin
    t <= a and b;
    y <= not t;
  end block;
end architecture;
