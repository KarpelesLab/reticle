library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity Top is
  port (
    clk : in std_logic;
    input : in std_logic;
    \entity\ : in std_logic;
    \a b\ : in unsigned(1 downto 0);
    \Foo\ : in unsigned(1 downto 0);
    \foo\ : out unsigned(1 downto 0);
    \9lives\ : out std_logic;
    \reticle_x\ : out std_logic;
    \_under\ : out std_logic;
    \a$b\ : out std_logic;
    \resize\ : out unsigned(3 downto 0);
    \q[0]\ : out std_logic;
    \mod\ : out std_logic
  );
end entity Top;

architecture rtl of Top is
  attribute \ram style\ : string;
  attribute keep : integer;
  alias clk_i : std_logic is clk;
  attribute \ram style\ of always : label is "block";
  attribute keep of \u.0\ : label is 1;
  component \vendor cell\ is
    generic (
      \INIT VAL\ : integer := 9
    );
    port (
      \in\ : in std_logic;
      \out port\ : in std_logic
    );
  end component;
begin
  \foo\ <= (\Foo\ and \a b\);
  \9lives\ <= (input or \entity\);
  \reticle_x\ <= \9lives\;
  \_under\ <= (not input);
  \a$b\ <= (input xor \entity\);
  \resize\ <= resize(\Foo\, 4);
  \q[0]\ <= \Foo\(0);
  always : process (clk_i)
  begin
    if rising_edge(clk_i) then
      \mod\ <= \entity\;
    end if;
  end process;
  \u.0\ : \vendor cell\ generic map (\INIT VAL\ => 9) port map (
    \in\ => input,
    \out port\ => \mod\
  );
end architecture rtl;
