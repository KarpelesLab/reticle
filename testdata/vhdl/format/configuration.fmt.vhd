library ieee;
use ieee.std_logic_1164.all;

entity adder is
  port (a, b : in bit; s, c : out bit);
end entity adder;

architecture gate of adder is
begin
  s <= a xor b;
  c <= a and b;
end architecture gate;

entity top is
end entity top;

architecture structural of top is
  component adder
    port (a, b : in bit; s, c : out bit);
  end component adder;
  signal x, y, s0, c0, s1, c1 : bit;
  for u0 : adder use entity work.adder(gate);
  for others : adder use entity work.adder port map (a => a, b => b, s => s, c => c);
begin
  u0 : adder port map (x, y, s0, c0);
  u1 : adder port map (a => x, b => y, s => s1, c => c1);
  g : for i in 0 to 1 generate
    ug : adder port map (x, y, open, open);
  end generate g;
end architecture structural;

configuration top_cfg of top is
  use work.all;
  for structural
    for u0 : adder
      use entity work.adder(gate);
    end for;
    for u1 : adder
      use configuration work.adder_cfg
        generic map (W => 1)
        port map (a => a, b => b, s => s, c => c);
    end for;
    for all : adder
      use open;
    end for;
    for g(0)
      for ug : adder
        use entity work.adder;
      end for;
    end for;
    for g(1 to 1)
      for others : adder
      end for;
    end for;
  end for;
end configuration top_cfg;

configuration adder_cfg of adder is
  for gate
  end for;
end configuration adder_cfg;
