library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity pll_wrap is
  port (
    clk_in : in std_logic;
    clk_out : out std_logic;
    locked : out std_logic;
    led : out std_logic
  );
end entity pll_wrap;

architecture rtl of pll_wrap is
  attribute keep : integer;
  attribute keep of pll : label is 1;
  component SB_PLL40_CORE is
    generic (
      DIVF : integer := 63;
      FEEDBACK_PATH : string := "SIMPLE"
    );
    port (
      REFERENCECLK : in std_logic;
      PLLOUTCORE : out std_logic;
      LOCK : out std_logic
    );
  end component;
  component vendor_blinky is
    port (
      clk : in std_logic;
      led : out std_logic
    );
  end component;
begin
  pll : SB_PLL40_CORE generic map (DIVF => 63, FEEDBACK_PATH => "SIMPLE") port map (
    REFERENCECLK => clk_in,
    PLLOUTCORE => clk_out,
    LOCK => locked
  );
  blink : vendor_blinky port map (
    clk => clk_out,
    led => led
  );
end architecture rtl;
