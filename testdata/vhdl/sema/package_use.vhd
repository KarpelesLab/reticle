-- A package with constants, a subtype and a procedure, used by a design.
package util_pkg is
  constant DATA_BITS : natural := 8;
  subtype byte_t is bit_vector(DATA_BITS - 1 downto 0);
  constant ALL_ONES : byte_t := (others => '1');

  procedure clear (signal b : out byte_t);
  procedure setbit (signal b : out bit; constant v : in bit);
end package util_pkg;

package body util_pkg is
  procedure clear (signal b : out byte_t) is
  begin
    b <= (others => '0');
  end procedure clear;

  procedure setbit (signal b : out bit; constant v : in bit) is
  begin
    b <= v;
  end procedure setbit;
end package body util_pkg;

use work.util_pkg.all;

entity pkg_user is
  port (
    go : in  bit;
    q  : out byte_t;
    f  : out bit
  );
end entity;

architecture rtl of pkg_user is
begin
  process (go)
  begin
    clear(q);
    setbit(f, go);
  end process;
end architecture;
