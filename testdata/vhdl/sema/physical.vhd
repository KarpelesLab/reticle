-- A user physical type next to the predefined `time`.
package phys_pkg is
  type distance is range 0 to 1000000000
    units
      um;
      mm = 1000 um;
      cm = 10 mm;
      m  = 100 cm;
    end units distance;

  constant SHORT : distance := 5 mm;
  constant LONG  : distance := 2 m;
  constant STEP  : time     := 5 ns;
  constant SLOW  : time     := STEP * 4;
end package phys_pkg;

use work.phys_pkg.all;

entity phys_user is
  port (d : out bit);
end entity;

architecture rtl of phys_user is
begin
  process
  begin
    d <= '1' after STEP;
    wait for SLOW;
    d <= '0';
    wait;
  end process;
end architecture;
