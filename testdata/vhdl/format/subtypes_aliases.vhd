library ieee;
use ieee.std_logic_1164.all;

package sub_pkg is
  subtype byte is std_logic_vector(7 downto 0);
  subtype nibble is bit_vector(3 downto 0);
  subtype small is integer range 0 to 15;
  subtype down is natural range 15 downto 0;
  subtype rbit is resolved std_logic;
  subtype rvec is resolved std_logic_vector(3 downto 0);
  subtype rvec_e is (resolved) std_logic_vector(3 downto 0);
  subtype idx is natural range byte'range;
  subtype word_arr is work.types_pkg.memory(0 to 3);
  subtype two_d is work.types_pkg.cube(1 to 1, 1 to 2, 1 to 1);
  subtype same is byte;

  alias byte_alias is byte;
  alias slv_and is "and" [std_logic_vector, std_logic_vector return std_logic_vector];
  alias to_int is work.util.to_integer [byte return integer];
  alias hi_lit is red [return color];
  alias \ext alias\ is \Extended Name\;
  alias 'z' is 'Z' [return std_logic];
  alias "&&" is "and" [bit, bit return bit];

  attribute enum_encoding : string;
  attribute max_delay : time;
  attribute syn_keep : boolean;
  attribute enum_encoding of color : type is "001 010 100";
  attribute max_delay of "and" [bit, bit return bit] : function is 1 ns;
  attribute syn_keep of others : signal is true;
  attribute syn_keep of all : variable is false;
  attribute syn_keep of a, b, 'c' : literal is true;
  attribute syn_keep of gen_label : label is true;
  attribute syn_keep of sub_pkg : package is true;
  attribute syn_keep of ent : entity is true;
  attribute syn_keep of arch : architecture is true;
  attribute syn_keep of cfg : configuration is true;
  attribute syn_keep of p : procedure is true;
  attribute syn_keep of t : subtype is true;
  attribute syn_keep of c : constant is true;
  attribute syn_keep of comp : component is true;
  attribute syn_keep of u : units is true;
  attribute syn_keep of g : group is true;
  attribute syn_keep of f : file is true;
end package sub_pkg;

architecture a of e is
  signal v : std_logic_vector(7 downto 0);
  alias top : std_logic is v(7);
  alias low : std_logic_vector(3 downto 0) is v(3 downto 0);
  alias rev : std_logic_vector(0 to 7) is v;
begin
end architecture a;
