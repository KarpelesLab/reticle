-- Array types, slices, indexing and index attributes.
package arr_pkg is
  type word_t is array (15 downto 0) of bit;
  type mem_t  is array (natural range <>) of word_t;
  subtype small_mem_t is mem_t(0 to 3);

  constant ZERO : word_t := (others => '0');
  constant ONE  : word_t := (0 => '1', others => '0');
end package arr_pkg;

use work.arr_pkg.all;

entity arr_user is
  port (
    d  : in  word_t;
    hi : out bit;
    lo : out bit
  );
end entity;

architecture rtl of arr_user is
  signal upper : bit_vector(7 downto 0);
  signal lower : bit_vector(7 downto 0);
  constant N   : natural := word_t'length;
  constant L   : integer := word_t'left;
  constant R   : integer := word_t'right;
  constant A   : boolean := word_t'ascending;
begin
  upper <= bit_vector(d(15 downto 8));
  lower <= bit_vector(d(7 downto 0));
  hi    <= d(d'high);
  lo    <= d(d'low);
end architecture;
