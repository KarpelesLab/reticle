LIBRARY ieee;
    USE ieee.std_logic_1164.ALL;

ENTITY ragged IS
        GENERIC (
  N : POSITIVE := 3 ;
            RESET_VALUE : std_logic := '0'
      ) ;
  PORT ( clk : IN std_logic ;
      rst : IN std_logic;
            en : IN std_logic ;
  q : OUT std_logic_vector ( N - 1 DOWNTO 0 )
        ) ;
END ENTITY ragged ;

ARCHITECTURE rtl OF ragged IS
      SIGNAL shifter : std_logic_vector ( N - 1 DOWNTO 0 ) ;
            SIGNAL parity : std_logic ;
  BEGIN
        PROCESS ( clk )
          BEGIN
      IF rising_edge ( clk ) THEN
            IF rst = '1' THEN
      shifter <= ( OTHERS => RESET_VALUE ) ;
                ELSIF en = '1' THEN
    shifter <= shifter ( N - 2 DOWNTO 0 ) & parity ;
          END IF ;
              END IF ;
    END PROCESS ;

          parity <= XOR shifter ;
                q <= shifter ;
END ARCHITECTURE ;
