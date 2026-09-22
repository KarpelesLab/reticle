context proj_ctx is
  library ieee;
  use ieee.std_logic_1164.all, ieee.numeric_std.all;
  context ieee.ieee_std_context;
end context proj_ctx;

library work;
context work.proj_ctx;

entity uses_ctx is
  port (a : in std_logic; y : out unsigned(3 downto 0));
end entity uses_ctx;
