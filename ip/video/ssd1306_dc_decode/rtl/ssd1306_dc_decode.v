// ssd1306_dc_decode — the tag an SSD1306 link's missing `dc` wire would have
// carried, worked out by reading the commands.
//
// `ip/bus/spi_display_rx` knows nothing about any controller, so it can only
// tag a byte by its position. This block knows the command set, so a driver
// is free to change what it sends:
//
//   a command        known length, so its arguments are never read as commands
//   0xB0..0xB7       a draw is next: the rest of this page, COLUMNS bytes when
//                    the column is 0
//   0x21 and 0x22    both complete, in either order: a draw of the window
//   anything else    a command, and the byte after it is a command too
//
// **The traffic that decided this is the user's device**: a 9-byte prefix
// (0xDB, 0xD5 and 0x81 with an argument each, then 0xA6, 0xC0, 0xA1) before
// every frame, then eight times one page command and 128 data bytes. That
// prefix "can vary on the device", so nothing here counts it.
//
// THE COMMAND TABLE IS QUOTED from the SSD1306 datasheet rev 1.1, section 10,
// and matches `ssd1306_slave`, plus the scroll commands 0x26, 0x27, 0x29,
// 0x2A and 0xA3, whose arguments could otherwise look like a page command.
// `ssd1306_slave` does not model those, so with or without a `dc` wire it
// reads their arguments as commands.
//
// A silence
//   IDLE_CLOCKS `clk` periods with no activity on `link_active`, after which
//   the next activity returns the block to reading commands, ending any draw
//   short. It takes effect at that activity and not at the threshold, because
//   a master holding its select through the delay closes its last byte after
//   the threshold, and that byte still belongs to the old draw. It must be
//   longer than anything inside a frame, **including the wait between a page
//   command and its data**. 0 turns the rule off, and a lost place is then lost
//   for good.
//
//   With the rule on, nothing is delivered until the first silence has been
//   seen, because a part that comes up mid-frame would otherwise read pixels as
//   commands, and some of those last: 0xAE turns the display off, and nothing
//   in this device's prefix would turn it on again. A part that comes up on
//   an idle line sees its silence IDLE_CLOCKS after reset, and a driver that
//   talks sooner loses what it sent until its next pause. `dropped_bytes` says
//   so. On a console's `z` this is what makes a reset safe mid-frame.
//
// What a command cannot say, and what happens then
//   - a page write that stops before the end of the page, or a window larger
//     than what is sent: the next commands are read as pixels until a silence.
//     `short_draws` counts it, at that silence.
//   - a window set once and then used for many frames with no command before
//     them: the pixels are read as commands, and `unknown_cmds` in
//     `ssd1306_slave` is what climbs.
//   - column commands after a page command: 0x00 could be either, so
//     PAGE_COLUMN_CMDS says how many there are. Before a page command they are
//     simply read. 0 is the device above; a wrong value moves every tag in the
//     page by that many bytes.
//   - 0x21 and 0x22 both sent during initialisation with more commands after
//     them: those commands are read as pixels, for one window's worth of bytes.
//   - a command this table does not have is taken to have no arguments.
//
// Not a transport: it takes bytes and gives bytes, one clock later. A silence
// clears its idea of the column but not the display's, so after a short draw a
// driver that means column 0 has to say so, as it would to a panel.
//
// Tested in `tests/ip_library.rs` and `examples/basys3/ssd1306_decode_tb.v`.
// Twenty hand mutations of this logic were run on 4 November 2026 against the
// former and all were caught. **Nothing here has run on a part**, and the
// real device's silence, which IDLE_CLOCKS must be set from, has not been
// measured.
module ssd1306_dc_decode #(
    parameter COLUMNS          = 128,
    parameter PAGES            = 8,
    // `clk` periods of silence that recover the place; 0 for never.
    parameter IDLE_CLOCKS      = 0,
    // Column commands sent after each page command: 0, 1 or 2.
    parameter PAGE_COLUMN_CMDS = 0,
    parameter COUNT_WIDTH      = 16
) (
    input  wire                   clk,
    input  wire                   rst_n,

    input  wire [7:0]             in_byte,
    input  wire                   in_valid,
    // High in any clock the link shows activity: `sclk_active` from
    // `spi_display_rx`. Not `in_valid`, which comes as a frame closes.
    input  wire                   link_active,

    output reg  [7:0]             out_byte,
    output reg                    out_is_data,
    output reg                    out_valid,

    // A silence has been seen, so bytes are being delivered.
    output wire                   synced,
    // The next byte is a pixel.
    output wire                   drawing,
    // Draws opened by a page command, and by a window.
    output wire [COUNT_WIDTH-1:0] page_draws,
    output wire [COUNT_WIDTH-1:0] window_draws,
    // Draws a silence ended early. **Should read zero.**
    output wire [COUNT_WIDTH-1:0] short_draws,
    // Bytes that arrived before the first silence. Zero on a part that came
    // up on an idle line.
    output wire [COUNT_WIDTH-1:0] dropped_bytes
);
    // Window arguments are cut to these widths, as `ssd1306_slave` cuts them:
    // drivers send a page end of 0xFF to mean "the last page".
    localparam integer COL_BITS  = (COLUMNS > 1) ? $clog2(COLUMNS) : 1;
    localparam integer PAGE_BITS = (PAGES > 1) ? $clog2(PAGES) : 1;
    localparam integer IDLE_W    = $clog2(IDLE_CLOCKS + 2);

    localparam [8:0]        COL_TOP   = COLUMNS - 1;
    localparam [IDLE_W-1:0] IDLE_FULL = IDLE_CLOCKS;
    localparam [IDLE_W-1:0] IDLE_ONE  = 1;
    localparam [1:0]        AFTER_PAGE = PAGE_COLUMN_CMDS;

    localparam [COUNT_WIDTH-1:0] CNT_ZERO = {COUNT_WIDTH{1'b0}};
    localparam [COUNT_WIDTH-1:0] CNT_MAX  = {COUNT_WIDTH{1'b1}};
    localparam [COUNT_WIDTH-1:0] CNT_ONE  = {{(COUNT_WIDTH-1){1'b0}}, 1'b1};

    reg                 synced_q;
    reg [IDLE_W-1:0]    idle_cnt;
    // In a draw, with this many more bytes in this page and pages after it.
    reg                 draw_q;
    reg [8:0]           cols_left;
    reg [4:0]           pages_left;
    reg [8:0]           cols_reload;
    // The command whose arguments are being read, and how many are to come.
    reg [7:0]           opcode;
    reg [2:0]           args_left;
    // Column commands still to come after a page command.
    reg [1:0]           after_page;
    // The column a page draw starts from.
    reg [7:0]           column;
    // Half of a window: which halves have been seen, and their sizes less one.
    reg                 have_cols;
    reg                 have_pages;
    reg [COL_BITS-1:0]  win_start;
    reg [8:0]           win_cols;
    reg [4:0]           win_pages;

    wire silent  = (IDLE_CLOCKS != 0) && (idle_cnt == IDLE_FULL);
    wire new_run = silent && link_active;
    wire take    = in_valid && synced_q;
    wire fresh   = !draw_q && (args_left == 3'd0);

    // The size of the half of a window this byte finishes.
    wire [8:0]           arg9      = {1'b0, in_byte};
    wire [COL_BITS-1:0]  cols_diff = arg9[COL_BITS-1:0] - win_start;
    wire [PAGE_BITS-1:0] page_diff = arg9[PAGE_BITS-1:0] - win_start[PAGE_BITS-1:0];
    wire [8:0]           cols_now  = cols_diff;
    wire [4:0]           pages_now = page_diff;

    wire ends_cols  = (args_left == 3'd1) && (opcode == 8'h21);
    wire ends_pages = (args_left == 3'd1) && (opcode == 8'h22);
    wire opens_window = !draw_q && ((ends_cols && have_pages)
                                 || (ends_pages && have_cols));

    // A column command moves this. So a page command, or the last column
    // command after one, sees where the draw starts in the same clock.
    wire is_col_lo = in_byte[7:4] == 4'h0;
    wire is_col_hi = in_byte[7:4] == 4'h1;
    wire is_page   = in_byte[7:3] == 5'b10110;
    wire [7:0] column_now = !fresh    ? column
                          : is_col_lo ? {column[7:4], in_byte[3:0]}
                          : is_col_hi ? {in_byte[3:0], column[3:0]}
                          :             column;
    wire [8:0] column9 = {1'b0, column_now};
    wire [8:0] page_rest = (column9 <= COL_TOP) ? (COL_TOP - column9) : 9'd0;

    wire opens_page = fresh && ((after_page == 2'd0) ? (is_page && AFTER_PAGE == 2'd0)
                                                     : (after_page == 2'd1));

    // The tag is the state before this byte: a page command is a command.
    wire is_data = draw_q;

    reg [COUNT_WIDTH-1:0] page_draws_q;
    reg [COUNT_WIDTH-1:0] window_draws_q;
    reg [COUNT_WIDTH-1:0] short_q;
    reg [COUNT_WIDTH-1:0] dropped_q;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            out_byte       <= 8'd0;
            out_is_data    <= 1'b0;
            out_valid      <= 1'b0;
            synced_q       <= (IDLE_CLOCKS == 0);
            idle_cnt       <= {IDLE_W{1'b0}};
            draw_q         <= 1'b0;
            cols_left      <= 9'd0;
            pages_left     <= 5'd0;
            cols_reload    <= 9'd0;
            opcode         <= 8'd0;
            args_left      <= 3'd0;
            after_page     <= 2'd0;
            column         <= 8'd0;
            have_cols      <= 1'b0;
            have_pages     <= 1'b0;
            win_start      <= {COL_BITS{1'b0}};
            win_cols       <= 9'd0;
            win_pages      <= 5'd0;
            page_draws_q   <= CNT_ZERO;
            window_draws_q <= CNT_ZERO;
            short_q        <= CNT_ZERO;
            dropped_q      <= CNT_ZERO;
        end else begin
            out_valid <= take;
            if (take) begin
                out_byte    <= in_byte;
                out_is_data <= is_data;
            end

            if (link_active) begin
                idle_cnt <= {IDLE_W{1'b0}};
            end else if (idle_cnt != IDLE_FULL) begin
                idle_cnt <= idle_cnt + IDLE_ONE;
            end
            if (silent) begin
                synced_q <= 1'b1;
            end
            if (in_valid && !synced_q && dropped_q != CNT_MAX) begin
                dropped_q <= dropped_q + CNT_ONE;
            end

            if (take) begin
                if (draw_q) begin
                    if (cols_left != 9'd0) begin
                        cols_left <= cols_left - 9'd1;
                    end else if (pages_left != 5'd0) begin
                        cols_left  <= cols_reload;
                        pages_left <= pages_left - 5'd1;
                    end else begin
                        draw_q <= 1'b0;
                        column <= 8'd0;
                    end
                end else if (args_left != 3'd0) begin
                    args_left <= args_left - 3'd1;
                    // The first argument of a window is its start.
                    if (args_left == 3'd2) begin
                        win_start <= arg9[COL_BITS-1:0];
                    end
                    if (ends_cols) begin
                        win_cols  <= cols_now;
                        have_cols <= 1'b1;
                    end
                    if (ends_pages) begin
                        win_pages  <= pages_now;
                        have_pages <= 1'b1;
                    end
                    if (opens_window) begin
                        draw_q      <= 1'b1;
                        cols_left   <= ends_cols ? cols_now : win_cols;
                        cols_reload <= ends_cols ? cols_now : win_cols;
                        pages_left  <= ends_cols ? win_pages : pages_now;
                        have_cols   <= 1'b0;
                        have_pages  <= 1'b0;
                        if (window_draws_q != CNT_MAX) begin
                            window_draws_q <= window_draws_q + CNT_ONE;
                        end
                    end
                end else begin
                    column <= column_now;
                    opcode <= in_byte;
                    if (after_page != 2'd0) begin
                        after_page <= after_page - 2'd1;
                    end else begin
                        casez (in_byte)
                            8'h20, 8'h81, 8'h8D, 8'hA8, 8'hD3, 8'hD5, 8'hD9,
                            8'hDA, 8'hDB:
                                args_left <= 3'd1;
                            8'h21, 8'h22, 8'hA3:
                                args_left <= 3'd2;
                            8'h29, 8'h2A:
                                args_left <= 3'd5;
                            8'h26, 8'h27:
                                args_left <= 3'd6;
                            8'b1011_0???:
                                after_page <= AFTER_PAGE;
                            default: ;
                        endcase
                    end
                    if (opens_page) begin
                        draw_q      <= 1'b1;
                        cols_left   <= page_rest;
                        cols_reload <= page_rest;
                        pages_left  <= 5'd0;
                        if (page_draws_q != CNT_MAX) begin
                            page_draws_q <= page_draws_q + CNT_ONE;
                        end
                    end
                end
            end

            // Last, so that it wins: this activity belongs to a new run, and
            // a byte that closes in this very clock has already been tagged.
            if (new_run) begin
                draw_q     <= 1'b0;
                args_left  <= 3'd0;
                after_page <= 2'd0;
                column     <= 8'd0;
                have_cols  <= 1'b0;
                have_pages <= 1'b0;
                if (draw_q && short_q != CNT_MAX) begin
                    short_q <= short_q + CNT_ONE;
                end
            end
        end
    end

    assign synced        = synced_q;
    assign drawing       = draw_q;
    assign page_draws    = page_draws_q;
    assign window_draws  = window_draws_q;
    assign short_draws   = short_q;
    assign dropped_bytes = dropped_q;
endmodule
