library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity counter_synth is
  port (
    clk : in std_logic;
    rst : in std_logic;
    en : in std_logic;
    q : out unsigned(3 downto 0);
    waddr : in unsigned(2 downto 0);
    wdata : in unsigned(7 downto 0);
    we : in std_logic;
    mem_out : out unsigned(7 downto 0)
  );
end entity counter_synth;

architecture rtl of counter_synth is
  type buf_t is array (0 to 7) of unsigned(7 downto 0);
  signal q_next : unsigned(3 downto 0);
  signal inc : unsigned(3 downto 0);
  signal carry : std_logic;
  signal sel : std_logic;
  signal lut_out : std_logic;
  signal buf : buf_t;
  constant lut0_init : unsigned(3 downto 0) := "0110";
  component SB_GB is
    port (
      USER_SIGNAL_TO_GLOBAL_BUFFER : in std_logic
    );
  end component;
begin
  inc <= (q + unsigned'("0001"));
  q_next <= inc when en = '1' else q;
  ff0 : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        q <= unsigned'("0000");
      else
        q <= q_next;
      end if;
    end if;
  end process;
  carry <= (or q);
  sel <= (carry and en);
  lut_out <= lut0_init(to_integer(unsigned'(sel & carry)));
  mem_out <= buf(to_integer(q(2 downto 0)));
  wr0 : process (clk)
  begin
    if rising_edge(clk) and we = '1' then buf(to_integer(waddr)) <= wdata; end if;
  end process;
  bb0 : SB_GB port map (
    USER_SIGNAL_TO_GLOBAL_BUFFER => lut_out
  );
end architecture rtl;
