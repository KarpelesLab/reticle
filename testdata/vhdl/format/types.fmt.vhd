package types_pkg is
  type color is (red, green, blue);
  type bit_char is ('0', '1', 'X', 'Z');
  type mixed is (a, 'b', c, \D\);
  type small_int is range 0 to 255;
  type down_int is range 10 downto -10;
  type voltage is range -1.0e3 to 1.0e3;
  type distance is range 0 to 1E9
    units
      um;
      mm = 1000 um;
      cm = 10 mm;
      m = 100 cm;
    end units distance;
  type duration is range 0 to integer'high
    units
      fs;
      ps = 1000 fs;
    end units duration;
  type word is array (15 downto 0) of bit;
  type memory is array (natural range <>) of word;
  type cube is array (1 to 2, 1 to 2, 1 to 2) of real;
  type ranged is array (small_int range <>) of bit;
  type by_enum is array (color) of natural;
  type by_range is array (color range red to green) of natural;
  type node;
  type node_ptr is access node;
  type node is record
    value     : integer;
    next_node : node_ptr;
  end record node;
  type int_file is file of integer;
  type text is file of string;
  type str_ptr is access string;
  type cstr_ptr is access string(1 to 10);
  subtype ptr_sub is node_ptr;
end package types_pkg;
