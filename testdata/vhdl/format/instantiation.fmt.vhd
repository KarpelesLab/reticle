library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity wrapper is
  port (clk : in std_logic; a : in unsigned(7 downto 0); y : out std_logic_vector(7 downto 0));
end entity wrapper;

architecture structural of wrapper is
  component reg
    generic (W : positive := 8; INIT : std_logic_vector := x"00");
    port (
      clk : in  std_logic;
      d   : in  std_logic_vector(W - 1 downto 0);
      q   : out std_logic_vector(W - 1 downto 0);
      en  : in  std_logic := '1'
    );
  end component reg;
  signal d, q : std_logic_vector(7 downto 0);
begin
  u_pos : reg port map (clk, d, q);
  u_named : reg
    generic map (W => 8, INIT => x"FF")
    port map (clk => clk, d => d, q => q, en => open);
  u_conv : reg port map (clk => clk, d => std_logic_vector(a), unsigned(q) => a, en => '1');
  u_partial : reg
    port map (clk => clk, d(3 downto 0) => d(7 downto 4), d(7 downto 4) => d(3 downto 0), q => q);
  u_comp_kw : reg port map (clk, d, q);
  u_ent : entity work.reg port map (clk, d, q);
  u_ent_arch : entity work.reg(rtl) generic map (8) port map (clk, d, q);
  u_cfg : configuration work.reg_cfg port map (clk => clk, d => d, q => q);
  u_bare : reg;
  u_lib : work.reg;
  u_fn : reg port map (clk => clk, d => to_slv(a, 8), q => q);
  y <= q;
end architecture structural;
