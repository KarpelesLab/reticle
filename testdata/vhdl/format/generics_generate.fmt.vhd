library ieee;
use ieee.std_logic_1164.all;

entity shift_reg is
  generic (N : positive := 4; MODE : string := "left"; INIT : std_logic_vector := x"0");
  port (clk : in std_logic; d : in std_logic; q : out std_logic_vector(N - 1 downto 0));
end entity shift_reg;

architecture gen of shift_reg is
  signal stage : std_logic_vector(N downto 0);
begin
  stage(0) <= d;

  chain : for i in 0 to N - 1 generate
    signal tap : std_logic;
  begin
    ff : process (clk)
    begin
      if rising_edge(clk) then
        tap <= stage(i);
      end if;
    end process ff;
    stage(i + 1) <= tap;
  end generate chain;

  dir : if MODE = "left" generate
    q <= stage(N downto 1);
  elsif MODE = "right" generate
    q <= stage(1 to N);
  else generate
    q <= (others => 'X');
  end generate dir;

  width : case N generate
    when one : 1 =>
      assert false report "single stage" severity note;
    when 2 | 3 =>
      assert false report "small" severity note;
    when others =>
      null_stmt : assert true;
  end generate width;

  nested : for r in 0 to 1 generate
    inner : for c in 0 to 1 generate
      dummy : block
      begin
      end block dummy;
    end generate inner;
  end generate nested;

  labelled : if alt : N > 8 generate
    assert false;
  end generate labelled;
end architecture gen;
