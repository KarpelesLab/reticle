library ieee;



use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity spaced is
  port (
    clk : in std_logic;

    a   : in std_logic;


    y   : out std_logic
  );
end entity spaced;
architecture rtl of spaced is


  signal t : std_logic;




  signal u : std_logic;
begin
  t <= a;


  u <= not t;
  y <= u;



end architecture rtl;



