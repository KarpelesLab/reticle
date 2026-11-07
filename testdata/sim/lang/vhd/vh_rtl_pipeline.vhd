library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package pipe_pkg is
  type stage_t is record
    valid : std_logic;
    a, b  : unsigned(7 downto 0);
    sum   : unsigned(8 downto 0);
    prod  : unsigned(15 downto 0);
  end record;
  constant STAGE_INIT : stage_t := (valid => '0', a => (others => '0'), b => (others => '0'),
                                    sum => (others => '0'), prod => (others => '0'));
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.pipe_pkg.all;

-- three-stage pipeline: register inputs, add, multiply sum by a
entity pipe is
  port (
    clk   : in std_logic;
    vin   : in std_logic;
    a, b  : in unsigned(7 downto 0);
    vout  : out std_logic;
    res   : out unsigned(15 downto 0)
  );
end entity;

architecture rtl of pipe is
  signal s1, s2, s3 : stage_t := STAGE_INIT;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      s1.valid <= vin;
      s1.a <= a;
      s1.b <= b;
      s2 <= s1;
      s2.sum <= resize(s1.a, 9) + s1.b;
      s3 <= s2;
      s3.prod <= resize(s2.sum * s2.a, 16);
    end if;
  end process;
  vout <= s3.valid;
  res <= s3.prod;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  signal clk : std_logic := '0';
  signal vin, vout : std_logic := '0';
  signal a, b : unsigned(7 downto 0) := (others => '0');
  signal res : unsigned(15 downto 0);
  signal stop : boolean := false;
  type int_arr is array (0 to 4) of integer;
  constant AV : int_arr := (1, 10, 200, 255, 7);
  constant BV : int_arr := (2, 20, 100, 255, 0);
begin
  clk <= not clk after 5 ns when not stop else clk;
  dut : entity work.pipe port map (clk => clk, vin => vin, a => a, b => b, vout => vout, res => res);
  process
    variable got, errs : integer := 0;
    variable exp : integer;
  begin
    for i in 0 to 7 loop
      if i <= 4 then
        a <= to_unsigned(AV(i), 8);
        b <= to_unsigned(BV(i), 8);
        vin <= '1';
      else
        vin <= '0';
      end if;
      wait until clk = '1';
      wait until clk = '0';
      if vout = '1' then
        exp := ((AV(got) + BV(got)) * AV(got)) mod 65536;
        if to_integer(res) /= exp then
          errs := errs + 1;
        end if;
        got := got + 1;
      end if;
    end loop;
    if got = 5 then report "pipeline valid count ok"; else report "pipeline valid count bad"; end if;
    if errs = 0 then report "pipeline results ok"; else report "pipeline results bad"; end if;
    stop <= true;
    wait;
  end process;
end architecture;
