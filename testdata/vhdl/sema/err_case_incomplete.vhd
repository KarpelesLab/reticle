-- Case statements that do not cover their selector, or repeat a choice.
entity err_case is
  port (
    n : in  natural range 0 to 3;
    q : out natural
  );
end entity;

architecture rtl of err_case is
  type mode_t is (off, slow, fast);
  signal mode : mode_t := off;
begin
  p1 : process (mode)
  begin
    case mode is
      when off  => q <= 0;
      when slow => q <= 1;
    end case;
  end process;

  p2 : process (n)
  begin
    case n is
      when 0      => q <= 0;
      when 1      => q <= 1;
      when 1      => q <= 2;
      when others => q <= 3;
    end case;
  end process;
end architecture;
