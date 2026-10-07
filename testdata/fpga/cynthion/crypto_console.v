// crypto_console — `ip/crypto/sha256` and `ip/crypto/chacha20` behind a
// line-oriented command interface, with a byte stream on each side.
//
// What it is for
//   Both blocks in `ip/crypto/` were written and verified entirely in
//   simulation: against FIPS 180-4, against RFC 8439 and against
//   `purecrypto`. Neither had ever run on a part, and both READMEs say so
//   and name the experiment they were not doing — a digest a host can
//   check with `sha256sum`, off a real die, at a clock that place and
//   route closed. This is that experiment, and
//   `testdata/fpga/cynthion/usb_crypto_console.v` is the board it runs on.
//
//   This module is **only the command interface**. It takes bytes from
//   somewhere and gives bytes back, and `crypto_console_ulpi.v` is what
//   puts `ip/usb/usb_cdc_acm` on the other side of them. The split is the
//   one `examples/mos6502_monitor` uses for the same reason: a parser is
//   software before it is gateware, and it deserves to be driven from a
//   test with the USB stack out of the way as well as through it.
//
// ===================================================================
// THE PROTOCOL, AND WHY IT IS THIS ONE
// ===================================================================
//
// A **line** is bytes up to a carriage return (0Dh) **or** a line feed
// (0Ah), and an **empty line is answered with nothing at all**. Those two
// rules together are what make CR, LF and CRLF all work with nothing having
// to agree about which: CRLF is one line and then an empty one, and the
// empty one is silent. It is also what makes
//
//     echo 'H abc' > /dev/ttyACM1
//
// — which sends a line feed and no carriage return — do what a person typing
// it plainly means, and which a console that insisted on CR would hang on.
//
// A space before the command letter is ignored too, for the same reason: a
// line a person typed with a stray space in front of it is a line they meant.
//
// Every other line gets exactly one answer line, terminated CR LF, and a
// short USB packet is committed at the end of it so a one-line answer does
// not wait for the next.
//
//   | Line | What it does | Answer |
//   |------|--------------|--------|
//   | `H <text>` | SHA-256 of the bytes between the separator and the CR | 64 hex digits |
//   | `h <hex>` | SHA-256 of the bytes written as hex | 64 hex digits |
//   | `E <hex>` | ChaCha20 of those bytes under the current key | hex, two digits a byte |
//   | `e <n>` | ChaCha20 of `n` zero bytes: the **keystream** | hex, two digits a byte |
//   | `Z <n>` | SHA-256 of `n` zero bytes made on the device | 64 hex digits |
//   | `X <n>` | SHA-256 of the keystream of `n` bytes | 64 hex digits |
//   | `K <64 hex>` | set the 256-bit key | `OK` |
//   | `N <24 hex>` | set the 96-bit nonce | `OK` |
//   | `C <8 hex>` | set the 32-bit block counter | `OK` |
//   | `k` / `n` / `c` | print the key, the nonce, the counter | hex |
//   | `?` | the command letters and what each takes | a help line |
//
//   Anything else, and any line whose argument is the wrong length or has
//   a character that is not hexadecimal in it, answers `ERR`. `n` above is
//   a hexadecimal count of bytes, one to eight digits.
//
// Why both a text form and a hex form, rather than one of them
//   Because the two are for different people and neither does the other's
//   job.
//
//   **`H <text>` is for a human at a terminal**, and that is half the value
//   of this design. `H abc` against `printf abc | sha256sum` is a check
//   anybody can do in one line with no script and nothing installed, and a
//   demonstration nobody can run is not a demonstration. It is also the
//   form that makes the device self-describing: `?` prints the letters, and
//   the first thing the port says after it enumerates is that same line, so
//   a person who opens it with `cat` knows what to type without reading
//   this file.
//
//   **`h <hex>` is for a test**, because the text form cannot express
//   every message and must not pretend to. A line ends at a CR or an LF, so a
//   message containing either cannot be typed; a terminal in the wrong mode
//   translates newlines, strips the eighth bit or eats control characters,
//   and the digest that comes back is then wrong about a message that was
//   never sent. Hex has none of those arguments: every byte of every
//   message is expressible, `h` with nothing after it is the **empty
//   message** — whose digest is a real value with no other way to ask for
//   it — and what the host meant is exactly what the device read.
//
//   The cipher has no text form at all, on purpose. A ChaCha20 output is
//   not printable, so the answer has to be hex whatever the input was, and
//   a protocol whose two sides are different shapes invites the reader to
//   get one of them wrong. `E` is hex in and hex out.
//
// Why there is no echo
//   A terminal that wants to show what is typed into it has its own echo,
//   and a device echo costs more than it looks. It doubles the bytes in the
//   only direction whose rate is worth measuring; it makes "did the digest
//   come back" and "did the input come back" the same observation when they
//   should be two; and with a one-byte-deep return path it deadlocks a host
//   that writes a whole line before reading any of it, which is what
//   `write_all` does. So nothing is echoed, and the thing that tells a
//   person the part is alive before anything has been typed is the
//   **banner**: the help line goes out the moment the host configures the
//   port, so `cat /dev/ttyACM1` says something on its own.
//
// Why there is no buffer, and what that buys
//   Nothing here holds a message. Bytes are decoded as they arrive and
//   handed straight to the core, and when the core is busy `rx_ready` goes
//   low and the USB OUT endpoint NAKs the host, which is what bulk flow
//   control is for. So **a line may be any length**: `h` followed by two
//   megabytes of hex digits on one line is a two-megabyte message, and
//   that is how the bytes-per-second figure in
//   `testdata/fpga/cynthion/usb_crypto_console.v`'s header was taken. It
//   also means `ip/memory/fifo_sync` is not needed, which saves having to
//   find out whether one can be placed — `usb_cdc_uart.v`'s header says it
//   could not be, and that note is now out of date.
//
// Why `Z` and `X` exist at all
//   To separate two rates that a single measurement confuses. `h` and `E`
//   measure the **link**: every byte crosses USB twice. `Z` and `X` make
//   their bytes on the device, so what they measure is the **core** —
//   SHA-256 alone, and ChaCha20 feeding SHA-256 — and the difference
//   between the two numbers is the answer to "what is the bottleneck".
//
//   `X` is also the strongest correctness check here, and it is nearly free
//   once `E` and `Z` exist: the digest of `n` bytes of keystream is a
//   64-digit answer that pins down every one of those `n` bytes, so a
//   megabyte of ChaCha20 output can be compared against `purecrypto` on the
//   host without a megabyte coming back over the wire.
//
// What it does not do
//   **No authentication.** `E` is ChaCha20 and nothing else: no Poly1305,
//   so a ciphertext from it is malleable and carries no tag.
//   `ip/crypto/chacha20/README.md` §6 is about exactly that.
//
//   **No nonce discipline.** The key, the nonce and the counter are
//   whatever was last typed, every `E`, `e` and `X` restarts the stream at
//   that counter, and two commands with the same three values produce the
//   same keystream — which for two different messages is the disaster
//   RFC 8439 §4 describes. This is an instrument for checking a block
//   against a specification, not a thing to encrypt with.
//
//   **No secrecy.** The key is typed over an unencrypted USB serial port
//   and `k` prints it back. That is the point of `k` and it is the opposite
//   of what a key wants.
//
//   **No length-range check.** `LEN_BITS` bits of byte counter is 4 GiB at
//   the default of 32, which is also the largest count `Z` and `X` can
//   express, so nothing this console can *ask* for overflows it. A `h`
//   line longer than that would, silently, and at the rate this link runs
//   that is hours of unbroken streaming.
//
//   **No mid-line recovery.** A `K`, `N` or `C` line with the wrong number
//   of digits answers `ERR` and leaves that one register holding the digits
//   that did arrive, shifted up. The registers are shifted into rather than
//   staged through a copy because a 256-bit staging register would cost
//   three hundred and eighty-four lookup tables of parallel load for nothing
//   this design needs, and `k`, `n` and `c` print the registers back, so a
//   disturbed one is visible rather than secret.
module crypto_console #(
    // Bits of the SHA-256 message byte counter. `ip/crypto/sha256`'s own
    // parameter; 32 is 4 GiB, and the paragraph above says why that is the
    // right width here rather than FIPS 180-4's 61.
    parameter integer LEN_BITS = 32
) (
    input  wire       clk,
    input  wire       rst_n,

    // Bytes from the host.
    input  wire [7:0] rx_data,
    input  wire       rx_valid,
    output wire       rx_ready,

    // Bytes to the host. `tx_commit` sends a short packet, which is what
    // keeps a one-line answer from waiting for the next one.
    output wire [7:0] tx_data,
    output wire       tx_valid,
    input  wire       tx_ready,
    output wire       tx_commit,

    // For the board's LEDs, and for a testbench that wants to see the
    // cores rather than infer them from the bytes.
    output wire       hash_busy,
    output wire       cipher_active,
    output wire       saw_line,
    output wire       saw_answer
);
    // -----------------------------------------------------------------
    // The characters this parser knows by name.
    // -----------------------------------------------------------------
    localparam [7:0] CH_CR = 8'h0D;
    localparam [7:0] CH_LF = 8'h0A;
    localparam [7:0] CH_SP = 8'h20;

    // -----------------------------------------------------------------
    // The state machine. Fourteen values, so four bits and not five.
    // -----------------------------------------------------------------
    localparam [3:0] S_CMD   = 4'd0;   // the first byte of a line
    localparam [3:0] S_TXT1  = 4'd1;   // `H`: the byte after the letter
    localparam [3:0] S_TXT   = 4'd2;   // `H`: the rest of the text
    localparam [3:0] S_HEX   = 4'd3;   // `h` / `E`: hex pairs
    localparam [3:0] S_NUM   = 4'd4;   // `e` / `Z` / `X`: a hex count
    localparam [3:0] S_ARG   = 4'd5;   // `K` / `N` / `C`: hex digits
    localparam [3:0] S_EOL1  = 4'd6;   // a command with no argument: the CR
    localparam [3:0] S_SKIP  = 4'd7;   // swallow to the CR, then answer ERR
    localparam [3:0] S_GEN   = 4'd8;   // making zero bytes
    localparam [3:0] S_FLUSH = 4'd9;   // draining the cipher stage
    localparam [3:0] S_LAST  = 4'd10;  // telling the hash the message ended
    localparam [3:0] S_WAIT  = 4'd11;  // waiting for the digest
    localparam [3:0] S_ANS   = 4'd12;  // emitting the answer
    localparam [3:0] S_EOL   = 4'd13;  // emitting CR, LF and the commit

    reg [3:0] state_q;

    // What the answer is made of. Six values, three bits.
    localparam [2:0] A_NONE = 3'd0;   // nothing but the CR LF
    localparam [2:0] A_DIG  = 3'd1;   // the digest
    localparam [2:0] A_KEY  = 3'd2;
    localparam [2:0] A_NON  = 3'd3;
    localparam [2:0] A_CTR  = 3'd4;
    localparam [2:0] A_STR  = 3'd5;   // a string out of the table below
    localparam [2:0] A_SB   = 3'd6;   // one streamed byte, two digits

    reg [2:0] ans_q;

    // Where each string starts in the table. The table is 51 bytes, so the
    // address is six bits and everything above it reads as the terminator.
    localparam [5:0] STR_ERR  = 6'd0;
    localparam [5:0] STR_OK   = 6'd4;
    localparam [5:0] STR_HELP = 6'd7;

    reg [5:0] str_q;

    // What the line asked for, decoded once at the command letter so that
    // nothing downstream has to remember which letter it was.
    reg use_hash_q;    // the bytes end up in `sha256`
    reg use_ciph_q;    // the bytes go through `chacha20` on the way
    // Which source a command reads from is `state_q` itself: `S_GEN` is the
    // counter and `S_TXT` / `S_HEX` are the host, so there is no third
    // register saying so.

    // -----------------------------------------------------------------
    // The string table. A `case` and not an indexed part-select of a
    // constant, for the reason `examples/mos6502_monitor/rtl/monitor_rom.v`
    // gives about its own: a `case` is a combinational process that
    // `synth::proc` lowers to a balanced multiplexer tree, and a tree is
    // what a 60 MHz clock can afford. A zero byte ends a string.
    // -----------------------------------------------------------------
    function [7:0] strtab;
        input [5:0] a;
        begin
            case (a)
                // "ERR"
                6'd0:  strtab = 8'h45;  // E
                6'd1:  strtab = 8'h52;  // R
                6'd2:  strtab = 8'h52;  // R
                // "OK"
                6'd4:  strtab = 8'h4F;  // O
                6'd5:  strtab = 8'h4B;  // K
                // "H t|h x|E x|e n|Z n|X n|K x|N x|C x|k|n|c|?"
                6'd7:  strtab = 8'h48;  // H
                6'd8:  strtab = 8'h20;  //
                6'd9:  strtab = 8'h74;  // t
                6'd10: strtab = 8'h7C;  // |
                6'd11: strtab = 8'h68;  // h
                6'd12: strtab = 8'h20;  //
                6'd13: strtab = 8'h78;  // x
                6'd14: strtab = 8'h7C;  // |
                6'd15: strtab = 8'h45;  // E
                6'd16: strtab = 8'h20;  //
                6'd17: strtab = 8'h78;  // x
                6'd18: strtab = 8'h7C;  // |
                6'd19: strtab = 8'h65;  // e
                6'd20: strtab = 8'h20;  //
                6'd21: strtab = 8'h6E;  // n
                6'd22: strtab = 8'h7C;  // |
                6'd23: strtab = 8'h5A;  // Z
                6'd24: strtab = 8'h20;  //
                6'd25: strtab = 8'h6E;  // n
                6'd26: strtab = 8'h7C;  // |
                6'd27: strtab = 8'h58;  // X
                6'd28: strtab = 8'h20;  //
                6'd29: strtab = 8'h6E;  // n
                6'd30: strtab = 8'h7C;  // |
                6'd31: strtab = 8'h4B;  // K
                6'd32: strtab = 8'h20;  //
                6'd33: strtab = 8'h78;  // x
                6'd34: strtab = 8'h7C;  // |
                6'd35: strtab = 8'h4E;  // N
                6'd36: strtab = 8'h20;  //
                6'd37: strtab = 8'h78;  // x
                6'd38: strtab = 8'h7C;  // |
                6'd39: strtab = 8'h43;  // C
                6'd40: strtab = 8'h20;  //
                6'd41: strtab = 8'h78;  // x
                6'd42: strtab = 8'h7C;  // |
                6'd43: strtab = 8'h6B;  // k
                6'd44: strtab = 8'h7C;  // |
                6'd45: strtab = 8'h6E;  // n
                6'd46: strtab = 8'h7C;  // |
                6'd47: strtab = 8'h63;  // c
                6'd48: strtab = 8'h7C;  // |
                6'd49: strtab = 8'h3F;  // ?
                default: strtab = 8'h00;
            endcase
        end
    endfunction

    // -----------------------------------------------------------------
    // Hexadecimal, in both directions.
    // -----------------------------------------------------------------
    wire       digit  = (rx_data >= 8'h30) && (rx_data <= 8'h39);
    wire       lower  = (rx_data >= 8'h61) && (rx_data <= 8'h66);
    wire       upper  = (rx_data >= 8'h41) && (rx_data <= 8'h46);
    wire       is_hex = digit | lower | upper;
    wire [3:0] nib    = digit ? rx_data[3:0] : (rx_data[3:0] + 4'd9);

    // **Either** terminator ends a line, and an empty line is answered with
    // nothing at all. That combination is what makes CR, LF and CRLF all
    // work with nothing having to agree about which, and it is what makes
    //
    //     echo 'H abc' > /dev/ttyACM1
    //
    // — which sends a line feed and no carriage return — do what a person
    // typing it plainly means. A console that took only CR would hang on it
    // and a console that took both and answered an empty line would send two
    // answers for one CRLF.
    wire is_cr  = (rx_data == CH_CR);
    wire is_lf  = (rx_data == CH_LF);
    wire is_eol = is_cr | is_lf;
    wire is_sp  = (rx_data == CH_SP);

    // Lower case out, because `sha256sum` prints lower case and a
    // comparison that has to fold case is a comparison with a step in it
    // that can be got wrong.
    function [7:0] hexchar;
        input [3:0] v;
        begin
            hexchar = (v < 4'd10) ? (8'h30 + {4'd0, v}) : (8'h57 + {4'd0, v});
        end
    endfunction

    // -----------------------------------------------------------------
    // The output byte, and the only place anything is given to the host.
    // -----------------------------------------------------------------
    reg [7:0] ob_q;
    reg       ob_full;
    assign tx_data  = ob_q;
    assign tx_valid = ob_full;
    // Room this cycle: either it is empty, or the byte in it is leaving
    // now. Loading it in the same cycle the old byte goes is what keeps the
    // emitters running back to back instead of every other cycle.
    wire ob_room = ~ob_full | tx_ready;

    // -----------------------------------------------------------------
    // THE REGISTERS THAT ARE ALSO THEIR OWN PRINTERS
    // -----------------------------------------------------------------
    // A hexadecimal register is loaded a nibble at a time from the left and
    // printed a nibble at a time from the left, which is the **same shift**
    // read two ways. So printing rotates: sixty-four rotations of a 256-bit
    // register by four bits put it back exactly where it started, and the
    // key survives being printed.
    //
    // That is worth stating because the obvious arrangement — one wide
    // staging register, parallel-loaded into three others — costs a
    // multiplexer per bit, three hundred and eighty-four of them. Rotating
    // in place costs the four bits where the nibble comes in and nothing
    // else, which is as close to free as a decision in this file gets.
    // `usb_crypto_console.v`'s header has what the whole console cost and
    // how much of the part is left.
    reg [255:0] key_q;
    reg [95:0]  nonce_q;
    reg [31:0]  ctr_q;

    // The digest is **not** copied anywhere. `ip/crypto/sha256` holds it on
    // its port until the next message replaces it, which its README §2 says
    // it spends 256 flip-flops to do, so a second copy here would be paying
    // for that twice. It is walked by selecting a nibble out of the port
    // instead, which is a sixty-four-way multiplexer four bits wide.
    reg [5:0] dig_ix_q;

    // A pulse of one cycle on either core's `start`. Registered, so it is
    // never in the same cycle as a byte: `ip/crypto/sha256`'s header asks a
    // caller not to raise `in_valid` with `start`, and `ip/crypto/chacha20`
    // asks the same, both for the same reason — `in_ready` is a function of
    // registers and can be high in the cycle the reload discards a byte.
    reg hs_q;
    reg cs_q;

    // One streamed byte, on its way out as two digits.
    reg [7:0] sb_q;

    // How many nibbles of the answer are still to come: up to 64, so seven
    // bits.
    reg [6:0] en_q;

    // How many hex digits an argument has had, **saturating at 65**: sixty-six
    // values, so seven bits.
    //
    // It saturates rather than wrapping because wrapping would be a defect a
    // person could type. A `K` line with 128 digits on it would take a
    // seven-bit counter back round to 64, which is the value that means
    // "exactly as many digits as the key has nibbles" — so a line twice as
    // long as it should be would be accepted and the key would be the
    // second half of what was typed. Held at 65, every length over 64 fails
    // every one of the checks below.
    reg [6:0] nb_q;
    wire [6:0] nb_next = (nb_q == 7'd65) ? nb_q : (nb_q + 7'd1);

    // Half a byte of a hex payload.
    reg [3:0] hi_q;
    reg       half_q;

    // How many bytes `Z`, `X` and `e` still have to make.
    reg [31:0] cnt_q;

    // Where in `S_EOL` we are: 2 is the CR, 1 is the LF, 0 is the commit.
    reg [1:0] eol_q;

    // -----------------------------------------------------------------
    // The cores.
    // -----------------------------------------------------------------
    wire [255:0] digest;
    wire         digest_valid;
    wire         sha_ready;
    reg          hash_feed;
    reg          hash_last;
    reg  [7:0]   hash_byte;

    sha256 #(
        .LEN_BITS (LEN_BITS)
    ) u_hash (
        .clk          (clk),
        .rst_n        (rst_n),
        .start        (hs_q),
        .in_byte      (hash_byte),
        .in_valid     (hash_feed),
        .in_last      (hash_last),
        .in_ready     (sha_ready),
        .digest_valid (digest_valid),
        .digest       (digest),
        .busy         (hash_busy)
    );

    wire [31:0] ciph_out;
    wire        ciph_out_valid;
    wire        ciph_ready;
    wire        ciph_exhausted;
    reg         ciph_valid;
    reg  [31:0] ciph_word;

    chacha20 u_cipher (
        .clk       (clk),
        .rst_n     (rst_n),
        .start     (cs_q),
        .key       (key_q),
        .nonce     (nonce_q),
        .counter   (ctr_q),
        .in_data   (ciph_word),
        .in_valid  (ciph_valid),
        .in_ready  (ciph_ready),
        .out_data  (ciph_out),
        .out_valid (ciph_out_valid),
        .active    (cipher_active),
        .exhausted (ciph_exhausted)
    );

    // -----------------------------------------------------------------
    // THE CIPHER'S THIRTY-TWO BIT PORT, WHICH IS A BYTE STREAM HERE
    // -----------------------------------------------------------------
    // `chacha20` moves 32 bits at a time because its core makes a block in
    // 22 cycles and a byte-wide port would take 64 — its README §3 has the
    // argument. This console is bytes at both ends, so four bytes are
    // packed into a word going in and the result is unpacked going out,
    // first byte at the most significant end, which is the order every
    // port of `ip/crypto/` uses.
    //
    // What it costs is a cycle count: a word is presented only when the
    // unpacker is empty, so four bytes take one cycle to accept, one for
    // the registered result and four to emit — six cycles for four bytes,
    // against the 16-in-39 the block itself can do. The console is not the
    // thing that makes this design slow (the USB link is, by two orders of
    // magnitude) and a second four-byte register to overlap them would
    // not change that, so it is not here.
    reg [23:0] pk_q;    // the one to three bytes held, right aligned
    reg [1:0]  pn_q;    // how many are held
    reg [31:0] up_q;    // the result being unpacked, next byte at the top
    reg [2:0]  un_q;    // how many of its bytes are still to come, 0 to 4
    reg [2:0]  nv_q;    // how many bytes the word in flight carried
    reg        infl_q;  // a word is in the cipher and its result has not come

    // A partial word, padded. The keystream depends on the key, the nonce
    // and the position and on nothing in `in_data`, so what the padding is
    // changes nothing — only `nv_q` decides how many of the four result
    // bytes are used.
    wire [31:0] pad_word = (pn_q == 2'd1) ? {pk_q[7:0],  24'd0}
                         : (pn_q == 2'd2) ? {pk_q[15:0], 16'd0}
                         :                  {pk_q[23:0],  8'd0};

    // -----------------------------------------------------------------
    // The source: where the next message byte comes from.
    // -----------------------------------------------------------------
    wire txt_byte_here = ((state_q == S_TXT) && rx_valid && !is_eol)
                       || ((state_q == S_TXT1) && rx_valid && !is_eol && !is_sp);
    wire hex_byte_here = (state_q == S_HEX) && rx_valid && is_hex && half_q;
    wire gen_byte_here = (state_q == S_GEN) && (cnt_q != 32'd0);

    wire       src_valid = txt_byte_here | hex_byte_here | gen_byte_here;
    wire [7:0] src_byte  = gen_byte_here ? 8'h00
                         : hex_byte_here ? {hi_q, nib}
                         :                 rx_data;

    // -----------------------------------------------------------------
    // The middle: the source, or the source through the cipher.
    // -----------------------------------------------------------------
    // The cipher stage will take a byte whenever it is not holding a full
    // word, and when it is, only if the word can move this cycle.
    wire word_room  = (un_q == 3'd0) && !infl_q;
    wire ciph_in_ok = ciph_ready & word_room;
    wire ciph_room  = (pn_q != 2'd3) | ciph_in_ok;

    wire       mid_valid = use_ciph_q ? (un_q != 3'd0) : src_valid;
    wire [7:0] mid_byte  = use_ciph_q ? up_q[31:24]    : src_byte;

    // -----------------------------------------------------------------
    // The sink: the hash, or two hex digits to the host.
    // -----------------------------------------------------------------
    wire mid_ready = use_hash_q ? sha_ready : (en_q == 7'd0);

    // And therefore what the source may hand over.
    wire src_ready = use_ciph_q ? ciph_room : mid_ready;

    // -----------------------------------------------------------------
    // Back-pressure to the host.
    // -----------------------------------------------------------------
    // Depending on `rx_data` is not a loop: every output of
    // `usb_bulk_ep`'s byte interface is a function of its own registers,
    // which that module's header states as a property of the block, so
    // nothing here reaches `rx_valid` combinationally.
    //
    // A carriage return, a line feed and the separator carry no message
    // byte, so they are taken whatever the cores are doing. Everything
    // else waits for room.
    assign rx_ready = (state_q == S_CMD)   ? 1'b1
                    : (state_q == S_SKIP)  ? 1'b1
                    : (state_q == S_EOL1)  ? 1'b1
                    : (state_q == S_NUM)   ? 1'b1
                    : (state_q == S_ARG)   ? 1'b1
                    : (state_q == S_TXT1)  ? (is_eol | is_sp | src_ready)
                    : (state_q == S_TXT)   ? (is_eol | src_ready)
                    : (state_q == S_HEX)   ? ((is_hex & half_q) ? src_ready : 1'b1)
                    :                        1'b0;

    wire rx_beat  = rx_valid & rx_ready;
    wire src_beat = src_valid & src_ready;
    wire mid_beat = mid_valid & mid_ready;

    // The digest's nibble, selected out of the core's own port. Written as
    // a `case` for the reason the string table is: a balanced tree, not a
    // sixty-four-deep chain of `if`s and not a barrel shifter over 256
    // bits.
    function [3:0] dignib;
        input [5:0] i;
        begin
            case (i)
                6'd0:  dignib = digest[255:252];  6'd1:  dignib = digest[251:248];
                6'd2:  dignib = digest[247:244];  6'd3:  dignib = digest[243:240];
                6'd4:  dignib = digest[239:236];  6'd5:  dignib = digest[235:232];
                6'd6:  dignib = digest[231:228];  6'd7:  dignib = digest[227:224];
                6'd8:  dignib = digest[223:220];  6'd9:  dignib = digest[219:216];
                6'd10: dignib = digest[215:212];  6'd11: dignib = digest[211:208];
                6'd12: dignib = digest[207:204];  6'd13: dignib = digest[203:200];
                6'd14: dignib = digest[199:196];  6'd15: dignib = digest[195:192];
                6'd16: dignib = digest[191:188];  6'd17: dignib = digest[187:184];
                6'd18: dignib = digest[183:180];  6'd19: dignib = digest[179:176];
                6'd20: dignib = digest[175:172];  6'd21: dignib = digest[171:168];
                6'd22: dignib = digest[167:164];  6'd23: dignib = digest[163:160];
                6'd24: dignib = digest[159:156];  6'd25: dignib = digest[155:152];
                6'd26: dignib = digest[151:148];  6'd27: dignib = digest[147:144];
                6'd28: dignib = digest[143:140];  6'd29: dignib = digest[139:136];
                6'd30: dignib = digest[135:132];  6'd31: dignib = digest[131:128];
                6'd32: dignib = digest[127:124];  6'd33: dignib = digest[123:120];
                6'd34: dignib = digest[119:116];  6'd35: dignib = digest[115:112];
                6'd36: dignib = digest[111:108];  6'd37: dignib = digest[107:104];
                6'd38: dignib = digest[103:100];  6'd39: dignib = digest[99:96];
                6'd40: dignib = digest[95:92];    6'd41: dignib = digest[91:88];
                6'd42: dignib = digest[87:84];    6'd43: dignib = digest[83:80];
                6'd44: dignib = digest[79:76];    6'd45: dignib = digest[75:72];
                6'd46: dignib = digest[71:68];    6'd47: dignib = digest[67:64];
                6'd48: dignib = digest[63:60];    6'd49: dignib = digest[59:56];
                6'd50: dignib = digest[55:52];    6'd51: dignib = digest[51:48];
                6'd52: dignib = digest[47:44];    6'd53: dignib = digest[43:40];
                6'd54: dignib = digest[39:36];    6'd55: dignib = digest[35:32];
                6'd56: dignib = digest[31:28];    6'd57: dignib = digest[27:24];
                6'd58: dignib = digest[23:20];    6'd59: dignib = digest[19:16];
                6'd60: dignib = digest[15:12];    6'd61: dignib = digest[11:8];
                6'd62: dignib = digest[7:4];      default: dignib = digest[3:0];
            endcase
        end
    endfunction

    wire [3:0] dig_nib = dignib(dig_ix_q);

    // The nibble that goes out next, by what the answer is made of.
    wire [3:0] nib_out = (ans_q == A_DIG) ? dig_nib
                       : (ans_q == A_KEY) ? key_q[255:252]
                       : (ans_q == A_NON) ? nonce_q[95:92]
                       : (ans_q == A_CTR) ? ctr_q[31:28]
                       :                    sb_q[7:4];

    // The answer's own byte, and whether there is one to give.
    wire       str_more  = (ans_q == A_STR) && (strtab(str_q) != 8'h00);
    wire       hex_more  = (ans_q != A_STR) && (ans_q != A_NONE) && (en_q != 7'd0);
    wire       ans_more  = str_more | hex_more;
    wire [7:0] ans_byte  = (ans_q == A_STR) ? strtab(str_q) : hexchar(nib_out);

    // Whether this cycle puts a byte into `ob_q`, and which byte. One
    // place, so that two emitters cannot both think they wrote it.
    wire emit_stream = (state_q != S_ANS) && (state_q != S_EOL)
                     && (ans_q == A_SB) && (en_q != 7'd0);
    wire emit_ans    = (state_q == S_ANS) && ans_more;
    wire emit_eol    = (state_q == S_EOL) && (eol_q != 2'd0);
    wire put         = ob_room & (emit_stream | emit_ans | emit_eol);
    wire [7:0] put_byte = emit_eol ? ((eol_q == 2'd2) ? CH_CR : CH_LF)
                        : emit_stream ? hexchar(sb_q[7:4])
                        : ans_byte;

    // Which emitter consumed a nibble or a table byte this cycle.
    wire step_hex = put & (emit_stream | (emit_ans & hex_more));
    wire step_str = put & emit_ans & str_more;

    // -----------------------------------------------------------------
    // The cores' inputs, as a function of the state and nothing else.
    // -----------------------------------------------------------------
    always @* begin
        hash_feed  = 1'b0;
        hash_last  = 1'b0;
        hash_byte  = mid_byte;
        ciph_valid = 1'b0;
        ciph_word  = (pn_q == 2'd3) ? {pk_q, src_byte} : pad_word;

        // A byte reaching the hash.
        if (use_hash_q) hash_feed = mid_valid;

        // The end of the message, which `ip/crypto/sha256` takes as
        // `in_last` with `in_valid` low — the one way to say "the message
        // ends here and there is no byte here", and the only way to
        // express the empty message at all.
        if (state_q == S_LAST) begin
            hash_feed = 1'b0;
            hash_last = 1'b1;
        end

        // A word reaching the cipher: the fourth byte of a full word while
        // the message runs, or whatever is left when it ends.
        if (use_ciph_q && word_room) begin
            if ((state_q == S_FLUSH) && (pn_q != 2'd0)) begin
                ciph_valid = 1'b1;
                ciph_word  = pad_word;
            end else if (src_valid && (pn_q == 2'd3)) begin
                ciph_valid = 1'b1;
                ciph_word  = {pk_q, src_byte};
            end
        end
    end

    assign tx_commit = (state_q == S_EOL) && (eol_q == 2'd0) && !ob_full;

    // -----------------------------------------------------------------
    // The latches the board's LEDs read.
    // -----------------------------------------------------------------
    reg saw_line_q;
    reg saw_answer_q;
    assign saw_line   = saw_line_q;
    assign saw_answer = saw_answer_q;

    // -----------------------------------------------------------------
    // Everything that moves.
    // -----------------------------------------------------------------
    // The reset state is `S_ANS` with the help line loaded, so the first
    // thing the port says is what to type at it. That is free — a state has
    // to reset to something — and it is what makes `cat /dev/ttyACM1`
    // enough to find out what this is. `crypto_console_ulpi.v` holds this
    // module in reset until the host has configured the port, for the
    // reason `examples/mos6502_monitor/rtl/monitor_ulpi.v` gives: a banner
    // printed into an endpoint a bus reset then clears is a banner nobody
    // sees.
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state_q    <= S_ANS;
            ans_q      <= A_STR;
            str_q      <= STR_HELP;
            use_hash_q <= 1'b0;
            use_ciph_q <= 1'b0;
            ob_q       <= 8'd0;
            ob_full    <= 1'b0;
            // RFC 8439 §2.4.2's key, nonce and counter, so that the first
            // `e 40` a person types can be compared against a published
            // hexdump without a key having to be typed first.
            key_q      <= 256'h000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f;
            nonce_q    <= 96'h000000000000004a00000000;
            ctr_q      <= 32'd1;
            dig_ix_q   <= 6'd0;
            sb_q       <= 8'd0;
            en_q       <= 7'd0;
            nb_q       <= 7'd0;
            hi_q       <= 4'd0;
            half_q     <= 1'b0;
            cnt_q      <= 32'd0;
            eol_q      <= 2'd2;
            pk_q       <= 24'd0;
            pn_q       <= 2'd0;
            up_q       <= 32'd0;
            un_q       <= 3'd0;
            nv_q       <= 3'd0;
            infl_q     <= 1'b0;
            hs_q       <= 1'b0;
            cs_q       <= 1'b0;
            saw_line_q   <= 1'b0;
            saw_answer_q <= 1'b0;
        end else begin
            // One cycle each, wherever they are raised below.
            hs_q <= 1'b0;
            cs_q <= 1'b0;

            // --------------------------------------------------------
            // The output byte.
            // --------------------------------------------------------
            if (put) begin
                ob_q    <= put_byte;
                ob_full <= 1'b1;
            end else if (ob_full & tx_ready) begin
                ob_full <= 1'b0;
            end

            // Written `q <= q | event` rather than `if (event) q <= 1'b1`
            // because the second infers a clock enable and a slice's two
            // flip-flops share one, which `clock_blink.v` explains.
            saw_line_q   <= saw_line_q | (rx_beat & is_eol);
            saw_answer_q <= saw_answer_q | tx_commit;

            // --------------------------------------------------------
            // The nibble and table pointers.
            // --------------------------------------------------------
            if (step_hex) begin
                en_q <= en_q - 7'd1;
                case (ans_q)
                    // Sixty-four rotations by four bits bring a 256-bit
                    // register back where it started, so printing the key
                    // does not destroy it.
                    A_KEY: key_q   <= {key_q[251:0],   key_q[255:252]};
                    A_NON: nonce_q <= {nonce_q[91:0],  nonce_q[95:92]};
                    A_CTR: ctr_q   <= {ctr_q[27:0],    ctr_q[31:28]};
                    A_DIG: dig_ix_q <= dig_ix_q + 6'd1;
                    default: sb_q  <= {sb_q[3:0], 4'd0};
                endcase
            end
            if (step_str) str_q <= str_q + 6'd1;

            // --------------------------------------------------------
            // The cipher stage.
            // --------------------------------------------------------
            if (ciph_valid & ciph_ready) begin
                // `nv_q` is how many of the four result bytes the caller
                // asked for: all of them mid-message, and `pn_q` on the
                // flush that ends one.
                nv_q   <= ((state_q == S_FLUSH) && (pn_q != 2'd0))
                              ? {1'b0, pn_q} : 3'd4;
                pn_q   <= 2'd0;
                infl_q <= 1'b1;
            end else if (src_beat && use_ciph_q && (pn_q != 2'd3)) begin
                pk_q <= {pk_q[15:0], src_byte};
                pn_q <= pn_q + 2'd1;
            end
            if (ciph_out_valid) begin
                up_q   <= ciph_out;
                un_q   <= nv_q;
                infl_q <= 1'b0;
            end else if (mid_beat & use_ciph_q) begin
                up_q <= {up_q[23:0], 8'd0};
                un_q <= un_q - 3'd1;
            end

            // --------------------------------------------------------
            // A byte reaching the hex sink, which is two digits.
            // --------------------------------------------------------
            if (mid_beat & ~use_hash_q) begin
                sb_q  <= mid_byte;
                en_q  <= 7'd2;
                ans_q <= A_SB;
            end

            // --------------------------------------------------------
            // The bytes the counter makes.
            // --------------------------------------------------------
            if (gen_byte_here & src_ready) cnt_q <= cnt_q - 32'd1;

            // --------------------------------------------------------
            // The parser.
            // --------------------------------------------------------
            case (state_q)
                S_CMD: begin
                    if (rx_valid) begin
                        use_hash_q <= 1'b0;
                        use_ciph_q <= 1'b0;
                        nb_q       <= 7'd0;
                        half_q     <= 1'b0;
                        cnt_q      <= 32'd0;
                        pn_q       <= 2'd0;
                        un_q       <= 3'd0;
                        infl_q     <= 1'b0;
                        ans_q      <= A_NONE;
                        // An empty line, or a space before the letter. The
                        // console says nothing, which is what makes one
                        // CRLF one answer.
                        if (is_eol | is_sp) begin
                            state_q <= S_CMD;
                        end else begin
                            case (rx_data)
                                8'h48: begin  // H: text, hashed
                                    use_hash_q <= 1'b1;
                                    state_q    <= S_TXT1;
                                end
                                8'h68: begin  // h: hex, hashed
                                    use_hash_q <= 1'b1;
                                    state_q    <= S_HEX;
                                end
                                8'h45: begin  // E: hex, enciphered
                                    use_ciph_q <= 1'b1;
                                    cs_q       <= 1'b1;
                                    state_q    <= S_HEX;
                                end
                                8'h65: begin  // e: keystream
                                    use_ciph_q <= 1'b1;
                                    cs_q       <= 1'b1;
                                    state_q    <= S_NUM;
                                end
                                8'h5A: begin  // Z: zeros, hashed
                                    use_hash_q <= 1'b1;
                                    state_q    <= S_NUM;
                                end
                                8'h58: begin  // X: keystream, hashed
                                    use_hash_q <= 1'b1;
                                    use_ciph_q <= 1'b1;
                                    cs_q       <= 1'b1;
                                    state_q    <= S_NUM;
                                end
                                8'h4B: begin  // K
                                    state_q <= S_ARG;
                                    ans_q   <= A_KEY;
                                end
                                8'h4E: begin  // N
                                    state_q <= S_ARG;
                                    ans_q   <= A_NON;
                                end
                                8'h43: begin  // C
                                    state_q <= S_ARG;
                                    ans_q   <= A_CTR;
                                end
                                8'h6B: begin  // k
                                    state_q <= S_EOL1;
                                    ans_q   <= A_KEY;
                                end
                                8'h6E: begin  // n
                                    state_q <= S_EOL1;
                                    ans_q   <= A_NON;
                                end
                                8'h63: begin  // c
                                    state_q <= S_EOL1;
                                    ans_q   <= A_CTR;
                                end
                                8'h3F: begin  // ?
                                    state_q <= S_EOL1;
                                    ans_q   <= A_STR;
                                    str_q   <= STR_HELP;
                                end
                                default: begin
                                    state_q <= S_SKIP;
                                end
                            endcase
                        end
                    end
                end

                // `H`'s first payload byte. One space here is the separator
                // and anything else is text, which is why this is a state
                // of its own: in `S_TXT` a space is part of the message.
                S_TXT1: begin
                    if (rx_beat) begin
                        if (is_eol) state_q <= S_FLUSH;
                        else        state_q <= S_TXT;
                    end
                end

                S_TXT: begin
                    if (rx_beat && is_eol) state_q <= S_FLUSH;
                end

                S_HEX: begin
                    if (rx_beat) begin
                        if (is_eol) begin
                            // Half a byte and then the end of the line is
                            // an odd number of digits, which is not a
                            // sequence of bytes.
                            if (half_q) begin
                                ans_q      <= A_STR;
                                str_q      <= STR_ERR;
                                use_hash_q <= 1'b0;
                                use_ciph_q <= 1'b0;
                                hs_q       <= 1'b1;
                                state_q    <= S_ANS;
                            end else begin
                                state_q <= S_FLUSH;
                            end
                        end else if (is_sp) begin
                            state_q <= S_HEX;
                        end else if (is_hex) begin
                            if (half_q) half_q <= 1'b0;
                            else begin
                                hi_q   <= nib;
                                half_q <= 1'b1;
                            end
                        end else begin
                            state_q    <= S_SKIP;
                            use_hash_q <= 1'b0;
                            use_ciph_q <= 1'b0;
                        end
                    end
                end

                S_NUM: begin
                    if (rx_beat) begin
                        if (is_eol) begin
                            if ((nb_q == 7'd0) || (nb_q > 7'd8)) begin
                                ans_q      <= A_STR;
                                str_q      <= STR_ERR;
                                use_hash_q <= 1'b0;
                                use_ciph_q <= 1'b0;
                                state_q    <= S_ANS;
                            end else begin
                                state_q <= S_GEN;
                            end
                        end else if (is_sp) begin
                            state_q <= S_NUM;
                        end else if (is_hex) begin
                            cnt_q <= {cnt_q[27:0], nib};
                            nb_q  <= nb_next;
                        end else begin
                            state_q    <= S_SKIP;
                            use_hash_q <= 1'b0;
                            use_ciph_q <= 1'b0;
                        end
                    end
                end

                S_ARG: begin
                    if (rx_beat) begin
                        if (is_eol) begin
                            // The register is the right one only if exactly
                            // as many digits arrived as it has nibbles.
                            if (((ans_q == A_KEY) && (nb_q == 7'd64))
                             || ((ans_q == A_NON) && (nb_q == 7'd24))
                             || ((ans_q == A_CTR) && (nb_q == 7'd8))) begin
                                ans_q <= A_STR;
                                str_q <= STR_OK;
                            end else begin
                                ans_q <= A_STR;
                                str_q <= STR_ERR;
                            end
                            state_q <= S_ANS;
                        end else if (is_sp) begin
                            state_q <= S_ARG;
                        end else if (is_hex) begin
                            nb_q <= nb_next;
                            case (ans_q)
                                A_KEY:   key_q   <= {key_q[251:0], nib};
                                A_NON:   nonce_q <= {nonce_q[91:0], nib};
                                default: ctr_q   <= {ctr_q[27:0], nib};
                            endcase
                        end else begin
                            ans_q   <= A_STR;
                            str_q   <= STR_ERR;
                            state_q <= S_SKIP;
                        end
                    end
                end

                S_EOL1: begin
                    if (rx_beat) begin
                        if (is_eol) begin
                            state_q <= S_ANS;
                            en_q    <= (ans_q == A_KEY) ? 7'd64
                                     : (ans_q == A_NON) ? 7'd24
                                     : (ans_q == A_CTR) ? 7'd8
                                     :                    7'd0;
                        end else begin
                            ans_q   <= A_STR;
                            str_q   <= STR_ERR;
                            state_q <= S_SKIP;
                        end
                    end
                end

                S_SKIP: begin
                    // Whatever was being built is abandoned. The hash is
                    // told so with `start`, which is what that port is for;
                    // the cipher does not need telling, because every
                    // command that uses it starts it.
                    if (rx_beat && is_eol) begin
                        ans_q   <= A_STR;
                        str_q   <= STR_ERR;
                        hs_q    <= 1'b1;
                        state_q <= S_ANS;
                    end
                end

                S_GEN: begin
                    if (cnt_q == 32'd0) state_q <= S_FLUSH;
                end

                S_FLUSH: begin
                    // Nothing left in the packer, nothing left in the
                    // unpacker and nothing left half-printed: everything
                    // the message was has reached the sink. The last of
                    // those three is not optional — a hex answer's final
                    // byte is two digits that have not gone out yet when
                    // the unpacker empties, and leaving without them would
                    // put the CR in front of them.
                    if ((!use_ciph_q
                         || ((pn_q == 2'd0) && (un_q == 3'd0) && !infl_q))
                        && (use_hash_q || (en_q == 7'd0))) begin
                        if (use_hash_q) begin
                            state_q <= S_LAST;
                        end else begin
                            state_q <= S_EOL;
                            eol_q   <= 2'd2;
                        end
                    end
                end

                S_LAST: begin
                    if (sha_ready) state_q <= S_WAIT;
                end

                S_WAIT: begin
                    if (digest_valid) begin
                        ans_q    <= A_DIG;
                        dig_ix_q <= 6'd0;
                        en_q     <= 7'd64;
                        state_q  <= S_ANS;
                    end
                end

                S_ANS: begin
                    if (!ans_more) begin
                        state_q <= S_EOL;
                        eol_q   <= 2'd2;
                    end
                end

                default: begin  // S_EOL
                    if (put) begin
                        eol_q <= eol_q - 2'd1;
                    end else if ((eol_q == 2'd0) && !ob_full) begin
                        state_q <= S_CMD;
                        ans_q   <= A_NONE;
                    end
                end
            endcase

            // The cipher's counter ran out, which `chacha20` answers by
            // refusing data for ever rather than repeating a keystream. A
            // console that waited for it would hang, and a person can reach
            // it by typing `C ffffffff` and then a long `e`, so it ends the
            // line with an error instead.
            if (ciph_exhausted && use_ciph_q
                && (state_q != S_ANS) && (state_q != S_EOL)) begin
                ans_q      <= A_STR;
                str_q      <= STR_ERR;
                use_hash_q <= 1'b0;
                use_ciph_q <= 1'b0;
                hs_q       <= 1'b1;
                state_q    <= S_ANS;
            end
        end
    end
endmodule
