-- top: tb
-- A testbench over `ieee.numeric_std` arithmetic. The unit under test is
-- an accumulator; the stimulus checks its numbers and then a batch of
-- operators whose answers are worked out by hand, so simulating this
-- design is a direct check of the native builtins.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity accum is
  port (
    clk : in  std_logic;
    rst : in  std_logic;
    inc : in  unsigned(7 downto 0);
    sum : out unsigned(7 downto 0)
  );
end entity;

architecture rtl of accum is
  signal acc : unsigned(7 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        acc <= (others => '0');
      else
        acc <= acc + inc;
      end if;
    end if;
  end process;

  sum <= acc;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  signal clk : std_logic := '0';
  signal rst : std_logic := '1';
  signal inc : unsigned(7 downto 0) := (others => '0');
  signal sum : unsigned(7 downto 0);
begin
  uut : entity work.accum port map (clk => clk, rst => rst, inc => inc, sum => sum);

  clocking : process
  begin
    for i in 0 to 7 loop
      wait for 5 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
    end loop;
    wait;
  end process;

  stimulus : process
    variable a : unsigned(7 downto 0);
    variable b : unsigned(7 downto 0);
    variable s : signed(7 downto 0);
    variable t : signed(7 downto 0);
  begin
    report "start";

    -- The accumulator: three increments of 10 from zero.
    inc <= to_unsigned(10, 8);
    wait for 12 ns;
    rst <= '0';
    wait for 30 ns;
    assert to_integer(sum) = 30 report "accumulator wrong" severity error;

    -- Unsigned arithmetic, including the wrap at 2**8.
    a := to_unsigned(200, 8);
    b := to_unsigned(100, 8);
    assert to_integer(a + b) = 44 report "unsigned add did not wrap" severity error;
    assert to_integer(a - b) = 100 report "unsigned sub wrong" severity error;
    assert to_integer(a / b) = 2 report "unsigned div wrong" severity error;
    assert to_integer(a rem b) = 0 report "unsigned rem wrong" severity error;
    assert to_integer(b - a) = 156 report "unsigned sub did not wrap" severity error;
    assert a > b report "unsigned compare wrong" severity error;

    -- Signed arithmetic: the sign of `rem` follows the dividend and the
    -- sign of `mod` follows the divisor, so the two differ here.
    s := to_signed(-7, 8);
    t := to_signed(3, 8);
    assert to_integer(s + t) = -4 report "signed add wrong" severity error;
    assert to_integer(s * t) = -21 report "signed mul wrong" severity error;
    assert to_integer(s / t) = -2 report "signed div wrong" severity error;
    assert to_integer(s rem t) = -1 report "signed rem wrong" severity error;
    assert to_integer(s mod t) = 2 report "signed mod wrong" severity error;
    assert to_integer(-s) = 7 report "signed negate wrong" severity error;
    assert to_integer(abs(s)) = 7 report "signed abs wrong" severity error;
    assert s < t report "signed compare wrong" severity error;

    -- Shifts: right of a signed value keeps the sign, of an unsigned
    -- value does not.
    assert to_integer(shift_right(s, 1)) = -4 report "signed shift wrong" severity error;
    assert to_integer(shift_left(b, 1)) = 200 report "unsigned shift wrong" severity error;
    assert to_integer(shift_right(a, 4)) = 12 report "unsigned shift wrong" severity error;

    -- Resizing: growing sign extends, narrowing a signed keeps the sign.
    assert to_integer(resize(s, 16)) = -7 report "resize wider wrong" severity error;
    assert to_integer(resize(b, 4)) = 4 report "resize narrower wrong" severity error;

    report "done";
    wait;
  end process;
end architecture;
