entity blocks is
  port (clk, en : in bit; y : out bit);
end entity;

architecture rtl of blocks is
  signal a, b : bit;
begin
  plain : block
  begin
    a <= clk;
  end block plain;

  guarded_blk : block (en = '1') is
    signal local : bit;
  begin
    local <= guarded a;
    y <= local;
  end block guarded_blk;

  mapped : block
    generic (W : positive := 1);
    generic map (W => 2);
    port (i : in bit; o : out bit);
    port map (i => a, o => b);
    constant K : integer := W;
  begin
    o <= i;
  end block mapped;

  outer : block
  begin
    inner : block
    begin
      b <= a;
    end block inner;
  end block outer;
end architecture rtl;
