-- A state machine over a user enumeration type: the state signal becomes
-- a two-bit net with the literal names in its `enum_literals` attribute.
library ieee;
use ieee.std_logic_1164.all;

entity fsm_enum is
  port (
    clk  : in  std_logic;
    rst  : in  std_logic;
    go   : in  std_logic;
    busy : out std_logic
  );
end entity;

architecture rtl of fsm_enum is
  type state_t is (idle, load, run, done);
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
  end process;

  next_logic : process (all)
  begin
    next_state <= state;
    case state is
      when idle =>
        if go = '1' then
          next_state <= load;
        end if;
      when load => next_state <= run;
      when run  => next_state <= done;
      when done => next_state <= idle;
    end case;
  end process;

  busy <= '0' when state = idle else '1';
end architecture;
