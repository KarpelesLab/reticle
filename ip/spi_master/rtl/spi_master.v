// spi_master — a byte-level SPI master in any of the four modes.
//
// What it does
//   One word per transfer, most significant bit first, full duplex:
//   pulse `start` with the word on `tx_data` and, WIDTH bit times later,
//   `done` is high for one cycle with what the slave shifted back on
//   `rx_data`. `busy` is high from the cycle `start` is accepted until
//   `done`.
//
//   CPOL is the level `sclk` rests at. CPHA chooses which of the two
//   `sclk` edges of a bit samples the input:
//
//     CPHA = 0  sample on the leading edge, change `mosi` on the
//               trailing one. The first bit is put on `mosi` when `cs_n`
//               falls, before any clock edge.
//     CPHA = 1  change `mosi` on the leading edge, sample on the
//               trailing one.
//
//   So (CPOL, CPHA) = (0,0) is mode 0, (0,1) mode 1, (1,0) mode 2 and
//   (1,1) mode 3. `sclk` is `clk` divided by 2 * CLK_DIV: CLK_DIV clock
//   cycles per half bit.
//
//   `cs_n` falls one half bit before the first `sclk` edge and rises one
//   half bit after the last one, so a slave that latches on `cs_n` sees a
//   clean frame with setup and hold either side of it.
//
// What it does not do
//   One slave: there is a single `cs_n` and no chip-select decoder, so a
//   bus with several devices needs the selects outside. No multi-byte
//   framing — `cs_n` rises between words, so a device wanting a command
//   byte and a data byte under one select needs a `cs_n` held outside
//   this block, which it cannot do; that is the honest limitation of a
//   byte-level master. No dual or quad IO, no DDR, no TI or National
//   frame formats, no `sclk` slower than `clk`/2 divided further than
//   CLK_DIV allows, and no input synchroniser on `miso`: it is sampled
//   directly, which is right for a source-synchronous slave clocked by
//   this very `sclk` and wrong for anything genuinely asynchronous.
module spi_master #(
    // Resting level of `sclk`.
    parameter CPOL    = 0,
    // 0 samples on the leading edge, 1 on the trailing edge.
    parameter CPHA    = 0,
    // Clock cycles per half bit; `sclk` period is twice this.
    parameter CLK_DIV = 4,
    // Bits per transfer, most significant first.
    parameter WIDTH   = 8
) (
    input  wire             clk,
    input  wire             rst_n,

    input  wire [WIDTH-1:0] tx_data,
    input  wire             start,
    output wire             busy,
    output reg              done,
    output reg  [WIDTH-1:0] rx_data,

    output wire             sclk,
    output reg              mosi,
    input  wire             miso,
    output wire             cs_n
);
    localparam CPOL_BIT  = (CPOL != 0) ? 1'b1 : 1'b0;
    localparam [15:0] DIV_LAST  = CLK_DIV - 1;
    // Two `sclk` edges per bit.
    localparam [7:0]  EDGE_LAST = (2 * WIDTH) - 1;

    reg         busy_q;
    reg         cs_n_q;
    reg         sclk_q;
    reg  [15:0] div_cnt;
    // Which `sclk` edge is next: even counts are leading edges.
    reg  [7:0]  edge_cnt;
    reg  [WIDTH-1:0] tx_shift;
    reg  [WIDTH-1:0] rx_shift;

    assign busy = busy_q;
    assign cs_n = cs_n_q;
    assign sclk = sclk_q;

    // What `rx_shift` becomes if this edge samples.
    wire [WIDTH-1:0] rx_next = {rx_shift[WIDTH-2:0], miso};
    // True on a leading edge, false on a trailing one.
    wire leading = !edge_cnt[0];
    // On a leading edge for CPHA = 1, and on a trailing edge for
    // CPHA = 0, the master drives the next bit.
    wire drive_edge = (CPHA != 0) ? leading : !leading;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            busy_q   <= 1'b0;
            cs_n_q   <= 1'b1;
            sclk_q   <= CPOL_BIT;
            div_cnt  <= 16'd0;
            edge_cnt <= 8'd0;
            tx_shift <= {WIDTH{1'b0}};
            rx_shift <= {WIDTH{1'b0}};
            rx_data  <= {WIDTH{1'b0}};
            mosi     <= 1'b0;
            done     <= 1'b0;
        end else begin
            done <= 1'b0;
            if (!busy_q) begin
                sclk_q   <= CPOL_BIT;
                div_cnt  <= 16'd0;
                edge_cnt <= 8'd0;
                if (start) begin
                    busy_q   <= 1'b1;
                    cs_n_q   <= 1'b0;
                    rx_shift <= {WIDTH{1'b0}};
                    // CPHA = 0 needs the first bit out before the first
                    // edge, so the shifter is loaded one place along;
                    // CPHA = 1 drives that bit on the first edge instead.
                    if (CPHA == 0) begin
                        mosi     <= tx_data[WIDTH-1];
                        tx_shift <= {tx_data[WIDTH-2:0], 1'b0};
                    end else begin
                        tx_shift <= tx_data;
                    end
                end else begin
                    cs_n_q <= 1'b1;
                end
            end else if (div_cnt == DIV_LAST) begin
                div_cnt  <= 16'd0;
                edge_cnt <= edge_cnt + 8'd1;

                if (edge_cnt == EDGE_LAST + 8'd1) begin
                    // One half bit after the last clock edge, so `cs_n`
                    // has a hold time and does not rise on the very edge
                    // the slave is still using.
                    busy_q <= 1'b0;
                    cs_n_q <= 1'b1;
                    done   <= 1'b1;
                end else begin
                    sclk_q <= !sclk_q;

                    if (drive_edge) begin
                        mosi     <= tx_shift[WIDTH-1];
                        tx_shift <= {tx_shift[WIDTH-2:0], 1'b0};
                    end else begin
                        rx_shift <= rx_next;
                    end

                    if (edge_cnt == EDGE_LAST) begin
                        // The last edge is a sampling edge exactly when
                        // it is not a driving one.
                        rx_data <= drive_edge ? rx_shift : rx_next;
                    end
                end
            end else begin
                div_cnt <= div_cnt + 16'd1;
            end
        end
    end
endmodule
