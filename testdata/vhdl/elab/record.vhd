-- A record is one wide net, the first field in the most significant bits;
-- selecting a field is a constant slice.
entity record_ports is
  port (
    d     : in  bit_vector(7 downto 0);
    valid : in  bit;
    hi    : out bit_vector(3 downto 0);
    ok    : out bit
  );
end entity;

architecture rtl of record_ports is
  type frame_t is record
    payload : bit_vector(7 downto 0);
    tag     : bit_vector(1 downto 0);
    valid   : bit;
  end record;

  signal frame : frame_t;
begin
  frame.payload <= d;
  frame.tag     <= "10";
  frame.valid   <= valid;

  hi <= frame.payload(7 downto 4);
  ok <= frame.valid;
end architecture;
