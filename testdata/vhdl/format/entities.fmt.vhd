library ieee;
use ieee.std_logic_1164.all;

entity empty_ent is
end entity empty_ent;

entity \Extended Name\ is
end entity \Extended Name\;

entity modes is
  generic (
    constant W : positive := 8;
    LATENCY    : natural range 0 to 3 := 1;
    NAME       : string := "modes";
    PERIOD     : time := 5 ns;
    SCALE      : real := 1.5e-3
  );
  port (
    signal clk : in      std_logic;
    a_in       : in      std_logic_vector(W - 1 downto 0) := (others => '0');
    b_out      : out     std_logic_vector(W - 1 downto 0);
    c_io       : inout   std_logic;
    d_buf      : buffer  std_logic;
    e_lnk      : linkage std_logic;
    f_bus      : in      std_logic bus;
    no_mode    :         std_logic
  );
  constant HALF : time := PERIOD / 2;
  signal internal : std_logic;
  attribute keep : boolean;
  attribute keep of internal : signal is true;
begin
  assert W > 0 report "W must be positive" severity error;
  passive : process (clk)
  begin
    assert clk /= 'X' report "clk is X";
  end process passive;
  check : postponed assert LATENCY <= 3;
end entity modes;

architecture empty of empty_ent is
begin
end architecture empty;
