library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
  port (
    ready : in std_logic;
    mode : in unsigned(1 downto 0);
    q : out unsigned(7 downto 0)
  );
end entity tb;

architecture rtl of tb is
  type mem_t is array (0 to 3) of unsigned(7 downto 0);
  signal clk : std_logic;
  signal rst_n : std_logic;
  signal d : unsigned(7 downto 0);
  signal cnt : unsigned(7 downto 0);
  signal i : unsigned(7 downto 0);
  signal j : unsigned(7 downto 0);
  signal tick : integer;
  signal hi : unsigned(3 downto 0);
  signal lo : unsigned(3 downto 0);
  signal \out\ : unsigned(3 downto 0);
  signal ok : std_logic;
  signal sense : unsigned(3 downto 0);
  signal addr : unsigned(1 downto 0);
  signal mem : mem_t;
begin
  ok <= \$onehot\(mode);
  reset_ff : process (clk, rst_n)
  begin
    if rst_n = '0' then
      q <= unsigned'("00000000");
    elsif rising_edge(clk) then
      q <= d;
    end if;
  end process;
  clock : process
    variable clk_v : std_logic;
  begin
    clk_v := clk;
    clk_v := '0';
    reticle_loop_1 : loop
      clk <= clk_v;
      wait for 5 ns;
      clk_v := clk;
      clk_v := (not clk_v);
    end loop reticle_loop_1;
    clk <= clk_v;
  end process;
  main : process
    variable rst_n_v : std_logic;
    variable d_v : unsigned(7 downto 0);
    variable cnt_v : unsigned(7 downto 0);
    variable i_v : unsigned(7 downto 0);
    variable j_v : unsigned(7 downto 0);
    variable tick_v : integer;
    variable hi_v : unsigned(3 downto 0);
    variable lo_v : unsigned(3 downto 0);
    variable reticle_tmp : unsigned(7 downto 0);
  begin
    rst_n_v := rst_n;
    d_v := d;
    cnt_v := cnt;
    i_v := i;
    j_v := j;
    tick_v := tick;
    hi_v := hi;
    lo_v := lo;
    rst_n_v := '0';
    d_v := unsigned'("00000000");
    cnt_v := unsigned'("00000000");
    tick_v := (tick_v - tick_v);
    report "start " & integer'image(to_integer(cnt_v));
    rst_n <= rst_n_v;
    d <= d_v;
    cnt <= cnt_v;
    i <= i_v;
    j <= j_v;
    tick <= tick_v;
    hi <= hi_v;
    lo <= lo_v;
    wait for 12 ns;
    rst_n_v := rst_n;
    d_v := d;
    cnt_v := cnt;
    i_v := i;
    j_v := j;
    tick_v := tick;
    hi_v := hi;
    lo_v := lo;
    rst_n_v := '1';
    rst_n <= rst_n_v;
    d <= d_v;
    cnt <= cnt_v;
    i <= i_v;
    j <= j_v;
    tick <= tick_v;
    hi <= hi_v;
    lo <= lo_v;
    wait until rising_edge(clk);
    rst_n_v := rst_n;
    d_v := d;
    cnt_v := cnt;
    i_v := i;
    j_v := j;
    tick_v := tick;
    hi_v := hi;
    lo_v := lo;
    i_v := unsigned'("00000000");
    reticle_loop_2 : while (i_v ?< unsigned'("00000100")) = '1' loop
      reticle_body_2 : loop
        if (i_v ?= unsigned'("00000010")) = '1' then
          exit reticle_body_2;
        end if;
        d_v := (d_v + i_v);
        mem(to_integer(i_v(1 downto 0))) <= d_v;
        rst_n <= rst_n_v;
        d <= d_v;
        cnt <= cnt_v;
        i <= i_v;
        j <= j_v;
        tick <= tick_v;
        hi <= hi_v;
        lo <= lo_v;
        wait until rising_edge(clk);
        rst_n_v := rst_n;
        d_v := d;
        cnt_v := cnt;
        i_v := i;
        j_v := j;
        tick_v := tick;
        hi_v := hi;
        lo_v := lo;
        exit reticle_body_2;
      end loop reticle_body_2;
      i_v := (i_v + unsigned'("00000001"));
    end loop reticle_loop_2;
    j_v := unsigned'("00000000");
    reticle_loop_3 : while (j_v ?< unsigned'("00001010")) = '1' loop
      j_v := (j_v + unsigned'("00000001"));
      if (j_v ?>= unsigned'("00000011")) = '1' then
        exit reticle_loop_3;
      end if;
    end loop reticle_loop_3;
    cnt_v := unsigned(to_signed(tick_v, 8));
    reticle_loop_5 : for reticle_i_4 in 1 to to_integer(unsigned'("00000010")) loop
      cnt_v := (cnt_v + unsigned'("00000001"));
    end loop reticle_loop_5;
    -- block named
    d <= transport unsigned'("00000111") after 2 ns;
    reticle_tmp := d_v;
    hi_v := reticle_tmp(7 downto 4);
    lo_v := reticle_tmp(3 downto 0);
    rst_n <= rst_n_v;
    d <= d_v;
    cnt <= cnt_v;
    i <= i_v;
    j <= j_v;
    tick <= tick_v;
    hi <= hi_v;
    lo <= lo_v;
    if not (ready = '1') then wait until ready = '1'; end if;
    rst_n_v := rst_n;
    d_v := d;
    cnt_v := cnt;
    i_v := i;
    j_v := j;
    tick_v := tick;
    hi_v := hi;
    lo_v := lo;
    assert (q ?= unsigned'("00000111")) = '1' report "q is " & to_hstring(q) & " not 7" severity error;
    assert '1' = '1' severity note;
    report "done";
    std.env.finish;
    std.env.stop;
    std.env.finish;
    rst_n <= rst_n_v;
    d <= d_v;
    cnt <= cnt_v;
    i <= i_v;
    j <= j_v;
    tick <= tick_v;
    hi <= hi_v;
    lo <= lo_v;
    wait;
  end process;
  decode : process (all)
    variable out_v : unsigned(3 downto 0);
  begin
    out_v := \out\;
    case mode is
      when "00" =>
        out_v := hi;
      when "01" | "10" =>
        out_v := lo;
      when others =>
        out_v := unsigned'("0000");
    end case;
    \out\ <= out_v;
  end process;
  wild : process (all)
    variable addr_v : unsigned(1 downto 0);
  begin
    addr_v := addr;
    case? d is
      when "1-------" =>
        addr_v := unsigned'("11");
      when "01XXXXXX" =>
        addr_v := unsigned'("10");
      when others =>
        addr_v := unsigned'("00");
    end case?;
    addr <= addr_v;
  end process;
  mixed : process (d, hi)
    variable sense_v : unsigned(3 downto 0);
  begin
    sense_v := sense;
    if d = q then
      sense_v := hi;
    elsif d = unsigned'("00000001") then
      sense_v := lo;
    end if;
    sense_v(0) := ready;
    sense <= sense_v;
  end process;
end architecture rtl;
