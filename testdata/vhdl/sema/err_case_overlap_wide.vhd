-- V0408: a choice inside an earlier range choice that spans most of
-- `integer`. The choices are kept as intervals; recording such a range one
-- value at a time never finished.
entity err_case_overlap_wide is
  port (n : in integer; q : out bit);
end entity;

architecture rtl of err_case_overlap_wide is
begin
  process (n)
  begin
    case n is
      when integer'low to 0 => q <= '0';
      when -5 => q <= '1';
      when 1 to integer'high => q <= '1';
    end case;
  end process;
end architecture;
