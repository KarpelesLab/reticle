library ieee;
use ieee.std_logic_1164.all;

entity traffic_light is
  port (
    clk    : in  std_logic;
    reset  : in  std_logic;
    button : in  std_logic;
    red    : out std_logic;
    amber  : out std_logic;
    green  : out std_logic
  );
end entity traffic_light;

architecture behav of traffic_light is
  type state_t is (s_red, s_red_amber, s_green, s_amber);
  signal state, next_state : state_t;
  attribute fsm_encoding : string;
  attribute fsm_encoding of state : signal is "one-hot";
begin
  sync : process (clk, reset)
  begin
    if reset = '1' then
      state <= s_red;
    elsif clk'event and clk = '1' then
      state <= next_state;
    end if;
  end process sync;

  comb : process (state, button)
  begin
    next_state <= state;
    case state is
      when s_red =>
        if button = '1' then
          next_state <= s_red_amber;
        end if;
      when s_red_amber => next_state <= s_green;
      when s_green => next_state <= s_amber;
      when s_amber => next_state <= s_red;
      when others => next_state <= s_red;
    end case;
  end process comb;

  with state select red <= '1' when s_red | s_red_amber, '0' when others;

  amber <= '1' when state = s_red_amber or state = s_amber else '0';
  green <= '1' when state = s_green else '0';
end architecture behav;
