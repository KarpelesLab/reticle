-- Case statements over enumerations, integers and vectors.
entity case_stmt is
  port (
    sel : in  bit_vector(1 downto 0);
    n   : in  natural range 0 to 3;
    q   : out natural
  );
end entity;

architecture rtl of case_stmt is
  type mode_t is (off, slow, fast);
  signal mode : mode_t := off;
begin
  enum_case : process (mode)
  begin
    case mode is
      when off  => q <= 0;
      when slow => q <= 1;
      when fast => q <= 2;
    end case;
  end process;

  int_case : process (n)
  begin
    case n is
      when 0      => q <= 10;
      when 1 | 2  => q <= 20;
      when 3      => q <= 30;
    end case;
  end process;

  vec_case : process (sel)
  begin
    case sel is
      when "00"   => q <= 0;
      when "01"   => q <= 1;
      when others => q <= 2;
    end case;
  end process;
end architecture;
