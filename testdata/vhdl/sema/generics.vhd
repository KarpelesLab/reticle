-- Generics of several types, with defaults and static folding.
entity gen is
  generic (
    WIDTH  : positive := 8;
    DEPTH  : natural  := 16;
    PERIOD : time     := 10 ns;
    RATIO  : real     := 0.5;
    NAME   : string   := "gen";
    DEBUG  : boolean  := false
  );
  port (
    d : in  bit_vector(7 downto 0);
    q : out bit_vector(7 downto 0)
  );
end entity gen;

architecture rtl of gen is
  constant TOTAL : natural := WIDTH * DEPTH;
  constant HALF  : real    := RATIO * 2.0;
  constant TWICE : time    := PERIOD * 2;
begin
  q <= d;

  assert TOTAL > 0
    report "empty " & NAME
    severity failure;
end architecture;
