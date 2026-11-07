library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  signal sel : std_logic := 'H';
  signal v : std_logic_vector(3 downto 0) := "1010";
begin
  process
    variable a : std_logic_vector(3 downto 0) := "1-0X";
    variable u : unsigned(3 downto 0) := "0101";
  begin
    report "1 ?= 1: " & std_logic'image('1' ?= '1');
    report "1 ?= H: " & std_logic'image('1' ?= 'H');
    report "0 ?= -: " & std_logic'image('0' ?= '-');
    report "1 ?= X: " & std_logic'image('1' ?= 'X');
    report "1 ?/= 0: " & std_logic'image('1' ?/= '0');
    report "v ?= 1-1-: " & std_logic'image(v ?= "1-1-");
    report "v ?= 0-1-: " & std_logic'image(v ?= "0-1-");
    report "v ?/= 1010: " & std_logic'image(v ?/= "1010");
    report "u ?< 6: " & std_logic'image(u ?< 6);
    report "u ?<= 5: " & std_logic'image(u ?<= 5);
    report "u ?> 5: " & std_logic'image(u ?> 5);
    report "u ?>= 5: " & std_logic'image(u ?>= 5);
    report "and v: " & std_logic'image(and v);
    report "or v: " & std_logic'image(or v);
    report "xor v: " & std_logic'image(xor v);
    report "nand v: " & std_logic'image(nand v);
    report "nor v: " & std_logic'image(nor v);
    report "xnor v: " & std_logic'image(xnor v);
    report "and 1111: " & std_logic'image(and std_logic_vector'("1111"));
    report "or a(with X): " & std_logic'image(or a);
    report "xor a: " & std_logic'image(xor a);
    if ?? sel then report "?? H true"; else report "?? H false"; end if;
    if sel then report "implicit ?? true"; else report "implicit ?? false"; end if;
    if v(1) and v(3) then report "implicit and true"; end if;
    if not v(0) then report "implicit not true"; end if;
    if (v ?= "1010") then report "match cond true"; end if;
    if ?? 'X' then report "?? X true"; else report "?? X false"; end if;
    report "maximum(3,9)=" & integer'image(maximum(3, 9));
    report "minimum(3,9)=" & integer'image(minimum(3, 9));
    report "minimum vec=" & to_string(minimum(std_logic_vector'("0110"), std_logic_vector'("0011")));
    wait;
  end process;
end architecture;
