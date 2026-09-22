library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity adder is
  port (
    a : in unsigned(3 downto 0);
    b : in unsigned(3 downto 0);
    y : out unsigned(3 downto 0)
  );
end entity adder;

architecture rtl of adder is
begin
  y <= (a + b);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity top is
  port (
    x : in unsigned(3 downto 0);
    y : in unsigned(3 downto 0);
    z : in unsigned(3 downto 0);
    s : out unsigned(3 downto 0)
  );
end entity top;

architecture rtl of top is
  signal t : unsigned(3 downto 0);
begin
  u0 : entity work.adder port map (
    a => x,
    b => y,
    y => t
  );
  u1 : entity work.adder port map (
    a => t,
    b => z,
    y => s
  );
end architecture rtl;
