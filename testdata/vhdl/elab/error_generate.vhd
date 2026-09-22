-- V0704: a generate bound that depends on a signal is not static after
-- elaboration.
entity error_generate is
  port (
    n : in  integer range 0 to 3;
    d : in  bit_vector(3 downto 0);
    q : out bit_vector(3 downto 0)
  );
end entity;

architecture rtl of error_generate is
begin
  g : for i in 0 to n generate
    q(i) <= not d(i);
  end generate;
end architecture;
