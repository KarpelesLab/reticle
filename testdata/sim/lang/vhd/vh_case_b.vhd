library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  type op_t is (NOP, LOAD, STORE, JUMP, HALT);
  function classify(n : integer) return integer is
  begin
    case n is
      when 0 => return 100;
      when 1 to 9 => return 101;
      when 10 | 20 | 30 => return 102;
      when others => return 104;
    end case;
  end function;
  function opclass(o : op_t) return integer is
  begin
    case o is
      when NOP => return 0;
      when LOAD | STORE => return 1;
      when JUMP to HALT => return 2;
    end case;
  end function;
  function dec(v : std_logic_vector(2 downto 0)) return integer is
  begin
    case v is
      when "000" => return 0;
      when "001" | "010" => return 1;
      when "1--" => return 7;   -- ordinary case: '-' matches only '-'
      when others => return 9;
    end case;
  end function;
  function decq(v : std_logic_vector(2 downto 0)) return integer is
  begin
    case? v is
      when "000" => return 0;
      when "01-" => return 2;
      when "1--" => return 4;
      when others => return 6;
    end case?;
  end function;
begin
  process
    variable u : unsigned(1 downto 0);
    variable r : integer;
  begin
    if classify(0) = 100 and classify(5) = 101 and classify(20) = 102 then report "int case ok"; else report "int case bad"; end if;
    if opclass(NOP) = 0 and opclass(STORE) = 1 and opclass(HALT) = 2 then report "enum case ok"; else report "enum case bad"; end if;
    if dec("000") = 0 and dec("010") = 1 and dec("110") = 9 then report "slv case ok"; else report "slv case bad"; end if;
    if dec("1--") = 7 then report "slv case dash literal ok"; else report "slv case dash literal bad"; end if;
    if decq("011") = 2 and decq("101") = 4 and decq("001") = 6 then report "case? ok"; else report "case? bad"; end if;
    if decq("1HL") = 4 then report "case? weak ok"; else report "case? weak bad"; end if;
    u := "10";
    case u is
      when "00" => r := 0;
      when "01" => r := 1;
      when "10" => r := 2;
      when others => r := 3;
    end case;
    if r = 2 then report "unsigned case ok"; else report "unsigned case bad"; end if;
    case to_integer(u) is
      when 2 => r := 22;
      when others => r := 0;
    end case;
    if r = 22 then report "expr case ok"; else report "expr case bad"; end if;
    case 'b' is
      when 'a' => r := 1;
      when 'b' => r := 2;
      when others => r := 3;
    end case;
    if r = 2 then report "char case ok"; else report "char case bad"; end if;
    wait;
  end process;
end architecture;
