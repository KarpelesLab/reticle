-- top: legacy
-- The Synopsys legacy packages. `std_logic_unsigned` gives
-- `std_logic_vector` unsigned arithmetic and `std_logic_arith` supplies
-- the `conv_*` conversions, both over the same native core as
-- `ieee.numeric_std`; the signedness of the first comes from the package,
-- not from the operand type.
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;

entity legacy is
  port (
    clk  : in  std_logic;
    rst  : in  std_logic;
    q    : out std_logic_vector(7 downto 0);
    wide : out std_logic_vector(11 downto 0);
    over : out std_logic
  );
end entity;

architecture rtl of legacy is
  signal cnt : std_logic_vector(7 downto 0);
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        cnt <= (others => '0');
      else
        cnt <= cnt + 1;
      end if;
    end if;
  end process;

  q    <= cnt;
  wide <= conv_std_logic_vector(conv_integer(cnt) * 3, 12);
  over <= '1' when cnt >= 250 else '0';
end architecture;

-- The sibling package makes the same operand types signed, which is the
-- only difference between the two: the comparison below is `lt` on a
-- signed pair, where the counter above compared unsigned.
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_signed.all;

entity legacy_signed is
  port (
    a   : in  std_logic_vector(7 downto 0);
    b   : in  std_logic_vector(7 downto 0);
    y   : out std_logic_vector(7 downto 0);
    neg : out std_logic;
    lss : out std_logic
  );
end entity;

architecture rtl of legacy_signed is
begin
  y   <= a - b;
  neg <= '1' when a < 0 else '0';
  lss <= '1' when a < b else '0';
end architecture;
