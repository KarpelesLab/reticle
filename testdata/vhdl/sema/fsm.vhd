-- A four-state machine over a user enumeration type.
library ieee;
use ieee.std_logic_1164.all;

entity fsm is
  port (
    clk   : in  std_logic;
    rst   : in  std_logic;
    go    : in  std_logic;
    busy  : out std_logic;
    done  : out std_logic
  );
end entity;

architecture rtl of fsm is
  type state_t is (idle, load, run, finish);
  signal state, next_state : state_t;
begin
  sync : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        state <= idle;
      else
        state <= next_state;
      end if;
    end if;
  end process sync;

  comb : process (state, go)
  begin
    next_state <= state;
    case state is
      when idle =>
        if go = '1' then
          next_state <= load;
        end if;
      when load   => next_state <= run;
      when run    => next_state <= finish;
      when finish => next_state <= idle;
    end case;
  end process comb;

  busy <= '0' when state = idle else '1';
  done <= '1' when state = finish else '0';
end architecture;
