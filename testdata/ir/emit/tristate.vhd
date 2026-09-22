library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity io_pad is
  port (
    oe : in std_logic;
    d : in unsigned(3 downto 0);
    pad : inout unsigned(3 downto 0);
    din : out unsigned(3 downto 0);
    \bit\ : in std_logic;
    bit_pad : inout std_logic
  );
end entity io_pad;

architecture rtl of io_pad is
begin
  pad <= d when oe = '1' else (others => 'Z');
  din <= pad;
  bit_pad <= \bit\ when oe = '1' else 'Z';
end architecture rtl;
