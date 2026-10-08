// A block RAM used as a ROM on a Digilent Basys 3: the eight right-hand
// slide switches are its address, eight LEDs show the word stored there.
//
// This is the smallest design that puts a RAMB18E1 on the part, with
// contents, and lets a person see whether the contents, the address and
// the data are in the right order. Every word holds its own address plus
// one, so there is exactly one right answer for every switch setting and
// almost every way of getting the block RAM wrong shows a different one.
//
// WHAT A PERSON SHOULD SEE: read SW7..SW0 as a binary number n (SW0 is
// the least significant bit, the right-most switch of the row). The eight
// LEDs LD14..LD7 then show n + 1 in binary, LD7 the least significant bit:
//
//   all switches down (n = 0)   ->  only LD7 lit         (1)
//   SW0 up            (n = 1)   ->  only LD8 lit         (2)
//   SW0 and SW1 up    (n = 3)   ->  only LD9 lit         (4)
//   SW7 up            (n = 128) ->  LD14 and LD7 lit     (129)
//   all eight up      (n = 255) ->  all eight dark       (256 wraps to 0)
//
// LD0..LD6, LD15 and SW8..SW15 are not part of the design. The LEDs follow
// the switches immediately: the read is clocked by the 100 MHz oscillator,
// so the delay is ten nanoseconds.
//
// What each failure looks like, which is why the contents are "n + 1"
// rather than something prettier:
//   - contents not loaded, or loaded into the other half of the block:
//     every LED dark for every setting;
//   - a wrong INIT bit order: the right number of LEDs lit, in the wrong
//     places (for n = 0, some LED other than LD7);
//   - a wrong or swapped address bit: n = 1, 2, 4, ... 128 each show the
//     value for some other n;
//   - the unused port writing over the memory (an input left at the
//     fabric's default one): the LEDs show 0xFF or 0x00 for one setting
//     and the right value for the others, or drift after power-up;
//   - the read port's own write enable left high: the word read back is
//     what the data pins carry, the same for every n.
//
// Why only LD7..LD14: eight LEDs in a row with nothing in between, and
// the row starts at LD7 because LD6 is ball U14, in the single-IOB tile
// at the bottom end of bank 14 (`LIOB33_SING_X0Y0`), which this flow
// could not configure when this design was written. It can now
// (`fpga::xray::TileAlias`) — the ball works and always did — and this
// design keeps its eight-in-a-row because that is what the reading
// procedure above describes.
//
// WHAT HAS BEEN TRIED: built by this flow to a bitstream in which every
// set bit decodes back through the database and the arcs equal the
// router's (tests/fpga_xray_bram.rs). Nothing has been loaded into a part.
// docs/fpga-xray.md, "Block RAM", has what is checked and what is quoted.
//
// There is no `+` in the logic: the "+ 1" is computed by the elaborator
// while it fills the memory, and the hardware is the block RAM alone.
module bram_rom (
    input  wire       clk,
    input  wire [7:0] sw,
    output reg  [7:0] led
);
    // Two kilobits: below the size the mapper puts in a block RAM by
    // itself, so the attribute asks for one.
    (* ram_style = "block" *)
    reg [7:0] rom [0:255];

    integer i;
    initial begin
        for (i = 0; i < 256; i = i + 1)
            rom[i] = i[7:0] + 8'd1;
    end

    always @(posedge clk)
        led <= rom[sw];
endmodule
