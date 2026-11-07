library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  signal data : std_logic_vector(31 downto 0) := x"12345678";
  signal swapped : std_logic_vector(31 downto 0);
begin
  -- byte swap in a combinational process with a loop-indexed slice
  swap : process (data)
  begin
    for i in 0 to 3 loop
      swapped(8 * i + 7 downto 8 * i) <= data(8 * (3 - i) + 7 downto 8 * (3 - i));
    end loop;
  end process;

  process
    variable sum : unsigned(7 downto 0);
    variable v : std_logic_vector(31 downto 0);
  begin
    wait for 1 ns;
    if swapped = x"78563412" then report "swap ok"; else report "swap bad"; end if;
    sum := (others => '0');
    for i in 0 to 3 loop
      sum := sum + unsigned(data(8 * i + 7 downto 8 * i));
    end loop;
    if sum = 16#14# then report "sum ok"; else report "sum bad"; end if;
    v := (others => '0');
    for i in 0 to 7 loop
      v(4 * i + 3 downto 4 * i) := std_logic_vector(to_unsigned(i, 4));
    end loop;
    if v = x"76543210" then report "nibble fill ok"; else report "nibble fill bad"; end if;
    for i in 0 to 3 loop
      v(i * 8 + 7 downto i * 8) := not v(i * 8 + 7 downto i * 8);
    end loop;
    if v = x"89ABCDEF" then report "invert ok"; else report "invert bad"; end if;
    data <= x"A1B2C3D4";
    wait for 1 ns;
    if swapped = x"D4C3B2A1" then report "swap2 ok"; else report "swap2 bad"; end if;
    wait;
  end process;
end architecture;
