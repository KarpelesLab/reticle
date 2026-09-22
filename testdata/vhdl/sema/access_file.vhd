-- Access types, incomplete types, allocators and file types.
package acc_pkg is
  type node;
  type node_ptr is access node;
  type node is record
    value : integer;
    next_node : node_ptr;
  end record;

  type int_file is file of integer;
  type str_ptr is access string;
end package acc_pkg;

use work.acc_pkg.all;

entity acc_user is
  port (q : out integer);
end entity;

architecture rtl of acc_user is
begin
  process
    variable head : node_ptr;
    variable tmp  : node_ptr;
    variable s    : str_ptr;
    variable total : integer := 0;
    file f : int_file;
  begin
    head := new node'(value => 1, next_node => null);
    tmp  := new node;
    s    := new string'("hello");
    total := head.value;
    deallocate(tmp);
    deallocate(head);
    deallocate(s);
    q <= total;
    wait;
  end process;
end architecture;
