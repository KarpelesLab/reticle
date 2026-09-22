-- V0707: access and file types have no synthesisable representation.
entity error_access is
  port (q : out integer);
end entity;

architecture rtl of error_access is
  type cell_t;
  type cell_ptr is access cell_t;
  type cell_t is record
    value : integer;
    next_cell : cell_ptr;
  end record;
begin
  process
    variable head : cell_ptr;
  begin
    head := new cell_t'(value => 1, next_cell => null);
    q <= head.value;
    wait;
  end process;
end architecture;
