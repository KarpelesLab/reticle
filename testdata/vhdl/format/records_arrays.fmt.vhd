library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package bus_pkg is
  type word_t is array (natural range <>) of std_logic_vector(31 downto 0);
  type mem_t is array (0 to 255) of std_logic_vector(7 downto 0);
  type matrix_t is array (natural range <>, natural range <>) of integer;
  type grid_t is array (0 to 3, 1 to 2) of bit;

  type axi_req_t is record
    addr     : std_logic_vector(31 downto 0);
    data     : std_logic_vector(31 downto 0);
    strb     : std_logic_vector(3 downto 0);
    valid    : std_logic;
    id, user : natural;
  end record axi_req_t;

  type axi_rsp_t is record
    data  : std_logic_vector(31 downto 0);
    resp  : std_logic_vector(1 downto 0);
    ready : std_logic;
  end record axi_rsp_t;

  -- VHDL-2008 partially constrained types and subtypes.
  type packet_t is record
    header  : std_logic_vector;
    payload : word_t;
  end record packet_t;
  subtype short_packet_t is packet_t(header(7 downto 0), payload(0 to 1)(15 downto 0));
  subtype open_words_t is word_t(open)(15 downto 0);

  constant REQ_INIT : axi_req_t :=
    (addr => (others => '0'), data => x"0000_0000", strb => "1111", valid => '0', id | user => 0);
  constant TABLE : mem_t := (0 => x"01", 1 to 3 => x"FF", others => x"00");
  constant M : matrix_t(0 to 1, 0 to 1) := ((1, 2), (3, 4));
end package bus_pkg;

entity slice_user is
  port (
    req : in  work.bus_pkg.axi_req_t;
    rsp : out work.bus_pkg.axi_rsp_t;
    v   : in  std_logic_vector(15 downto 0)
  );
end entity slice_user;

architecture rtl of slice_user is
  signal lo, hi : std_logic_vector(7 downto 0);
  signal words : work.bus_pkg.word_t(0 to 3);
  alias top_bit : std_logic is v(15);
  alias low_byte : std_logic_vector(7 downto 0) is v(7 downto 0);
begin
  lo <= v(7 downto 0);
  hi <= v(v'high downto 8);
  rsp.data <= req.addr(31 downto 16) & req.data(15 downto 0);
  rsp.data(3 downto 0) <= req.strb;
  rsp.resp <= "00";
  rsp.ready <= req.valid;
  words(0) <= req.addr;
  words(1)(7 downto 0) <= lo;
  words(2 to 3) <= (others => (others => '0'));
  (lo, hi) <= v;
end architecture rtl;
