library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package uart_pkg is
  constant DATA_BITS : natural := 8;
  constant STOP_BITS : natural := 1;

  type uart_state_t is (idle, start, data, stop);

  subtype byte_t is std_logic_vector(DATA_BITS - 1 downto 0);

  function baud_divisor(clk_hz : natural; baud : natural) return natural;
  procedure send_byte(signal tx : out std_logic; constant value : in byte_t);

  component uart_tx is
    generic (DIVISOR : positive);
    port (
      clk   : in  std_logic;
      rst_n : in  std_logic;
      din   : in  byte_t;
      valid : in  std_logic;
      ready : out std_logic;
      tx    : out std_logic
    );
  end component uart_tx;
end package uart_pkg;

package body uart_pkg is
  function baud_divisor(clk_hz : natural; baud : natural) return natural is
  begin
    return (clk_hz + baud / 2) / baud;
  end function baud_divisor;

  procedure send_byte(signal tx : out std_logic; constant value : in byte_t) is
  begin
    tx <= '0';
    for i in value'range loop
      tx <= value(i);
    end loop;
    tx <= '1';
  end procedure;
end package body uart_pkg;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.uart_pkg.all;

entity uart_tx is
  generic (DIVISOR : positive);
  port (
    clk   : in  std_logic;
    rst_n : in  std_logic;
    din   : in  byte_t;
    valid : in  std_logic;
    ready : out std_logic;
    tx    : out std_logic
  );
end entity;

architecture rtl of uart_tx is
  signal state    : uart_state_t := idle;
  signal shift    : byte_t;
  signal bit_idx  : natural range 0 to DATA_BITS - 1;
  signal baud_cnt : natural range 0 to DIVISOR - 1;
  signal tick     : std_logic;
begin
  baud : process (clk, rst_n) is
  begin
    if rst_n = '0' then
      baud_cnt <= 0;
      tick     <= '0';
    elsif rising_edge(clk) then
      if baud_cnt = DIVISOR - 1 then
        baud_cnt <= 0;
        tick     <= '1';
      else
        baud_cnt <= baud_cnt + 1;
        tick     <= '0';
      end if;
    end if;
  end process baud;

  fsm : process (clk, rst_n)
    variable next_idx : natural;
  begin
    if rst_n = '0' then
      state <= idle;
      tx    <= '1';
    elsif rising_edge(clk) then
      case state is
        when idle =>
          tx <= '1';
          if valid = '1' then
            shift   <= din;
            bit_idx <= 0;
            state   <= start;
          end if;
        when start =>
          if tick = '1' then
            tx    <= '0';
            state <= data;
          end if;
        when data =>
          if tick = '1' then
            tx       <= shift(bit_idx);
            next_idx := bit_idx + 1;
            if next_idx = DATA_BITS then
              state <= stop;
            else
              bit_idx <= next_idx;
            end if;
          end if;
        when stop =>
          if tick = '1' then
            tx    <= '1';
            state <= idle;
          end if;
      end case;
    end if;
  end process fsm;

  ready <= '1' when state = idle else '0';
end architecture rtl;
