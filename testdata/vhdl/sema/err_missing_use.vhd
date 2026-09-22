-- `std_logic` without the use clause that declares it.
entity err_missing_use is
  port (
    clk : in  std_logic;
    q   : out std_logic_vector(7 downto 0)
  );
end entity;
