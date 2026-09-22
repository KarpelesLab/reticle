-- The STD.TEXTIO package (IEEE 1076-2008 clause 16.4).
--
-- Clean-room source for Reticle. The visible declarations are those the
-- standard prescribes. Every subprogram here touches the host's files or
-- formats values, which the sans-I/O analyser cannot do, so each is marked
-- `attribute foreign` and implemented natively by the simulator; the
-- package therefore has no body.

package textio is

  type line is access string;

  type text is file of string;

  type side is (right, left);

  subtype width is natural;

  -- reticle: builtin -- the simulator binds these to the host's streams.
  function justify (value     : string;
                    justified : side  := right;
                    field     : width := 0) return string;

  file input  : text open read_mode  is "STD_INPUT";
  file output : text open write_mode is "STD_OUTPUT";

  procedure readline (file f : text; l : inout line);
  procedure writeline (file f : text; l : inout line);
  procedure tee (file f : text; l : inout line);

  procedure read (l : inout line; value : out bit;        good : out boolean);
  procedure read (l : inout line; value : out bit);
  procedure read (l : inout line; value : out bit_vector; good : out boolean);
  procedure read (l : inout line; value : out bit_vector);
  procedure read (l : inout line; value : out boolean;    good : out boolean);
  procedure read (l : inout line; value : out boolean);
  procedure read (l : inout line; value : out character;  good : out boolean);
  procedure read (l : inout line; value : out character);
  procedure read (l : inout line; value : out integer;    good : out boolean);
  procedure read (l : inout line; value : out integer);
  procedure read (l : inout line; value : out real;       good : out boolean);
  procedure read (l : inout line; value : out real);
  procedure read (l : inout line; value : out string;     good : out boolean);
  procedure read (l : inout line; value : out string);
  procedure read (l : inout line; value : out time;       good : out boolean);
  procedure read (l : inout line; value : out time);

  procedure sread (l : inout line; value : out string; strlen : out natural);

  procedure write (l         : inout line;
                   value     : in bit;
                   justified : in side  := right;
                   field     : in width := 0);
  procedure write (l         : inout line;
                   value     : in bit_vector;
                   justified : in side  := right;
                   field     : in width := 0);
  procedure write (l         : inout line;
                   value     : in boolean;
                   justified : in side  := right;
                   field     : in width := 0);
  procedure write (l         : inout line;
                   value     : in character;
                   justified : in side  := right;
                   field     : in width := 0);
  procedure write (l         : inout line;
                   value     : in integer;
                   justified : in side  := right;
                   field     : in width := 0);
  procedure write (l         : inout line;
                   value     : in real;
                   justified : in side  := right;
                   field     : in width := 0;
                   digits    : in natural := 0);
  procedure write (l         : inout line;
                   value     : in string;
                   justified : in side  := right;
                   field     : in width := 0);
  procedure write (l         : inout line;
                   value     : in time;
                   justified : in side  := right;
                   field     : in width := 0;
                   unit      : in time := ns);

  -- VHDL-2008: radix-aware reads and writes of bit vectors.
  procedure hread (l : inout line; value : out bit_vector; good : out boolean);
  procedure hread (l : inout line; value : out bit_vector);
  procedure oread (l : inout line; value : out bit_vector; good : out boolean);
  procedure oread (l : inout line; value : out bit_vector);
  procedure bread (l : inout line; value : out bit_vector; good : out boolean);
  procedure bread (l : inout line; value : out bit_vector);

  procedure hwrite (l         : inout line;
                    value     : in bit_vector;
                    justified : in side  := right;
                    field     : in width := 0);
  procedure owrite (l         : inout line;
                    value     : in bit_vector;
                    justified : in side  := right;
                    field     : in width := 0);
  procedure bwrite (l         : inout line;
                    value     : in bit_vector;
                    justified : in side  := right;
                    field     : in width := 0);

  attribute foreign of justify  : function  is "reticle: builtin";
  attribute foreign of readline : procedure is "reticle: builtin";
  attribute foreign of writeline: procedure is "reticle: builtin";
  attribute foreign of tee      : procedure is "reticle: builtin";
  attribute foreign of read     : procedure is "reticle: builtin";
  attribute foreign of sread    : procedure is "reticle: builtin";
  attribute foreign of write    : procedure is "reticle: builtin";
  attribute foreign of hread    : procedure is "reticle: builtin";
  attribute foreign of oread    : procedure is "reticle: builtin";
  attribute foreign of bread    : procedure is "reticle: builtin";
  attribute foreign of hwrite   : procedure is "reticle: builtin";
  attribute foreign of owrite   : procedure is "reticle: builtin";
  attribute foreign of bwrite   : procedure is "reticle: builtin";

end package textio;
