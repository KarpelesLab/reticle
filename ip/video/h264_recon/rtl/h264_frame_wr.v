// h264_frame_wr — the decoded picture's way into SDRAM: row records
// from `h264_deblock` in, bursts on an `sdram_burst` client port out.
//
// What it does
//   Holds up to 64 records from the deblocker in a queue and turns each
//   header and the words after it into one write burst: a luma row
//   segment of 16 samples is 8 SDRAM words (cmd_len 7), a chroma one of
//   8 samples is 4 (cmd_len 3). Two samples a word, the even one in bits
//   7:0, which is the tiled layout's packing; each 32-bit record word is
//   two beats, its low half first.
//
//   A segment never leaves its tile: it starts at a multiple of 16
//   (luma) or 8 (chroma) samples, and a tile row is 32 samples, so a
//   burst stays inside one 512-word SDRAM row as the port requires.
//
//   `pic_done` pulses once the end-of-picture record has been reached,
//   which is after every burst of the picture has had its last beat
//   accepted.
//
// Where a sample goes
//   `frame_addr` of `ip/memory/sdram_burst` maps (frame, plane, x, y) to
//   the word address, as every client of the frame layout does.
module h264_frame_wr (
    input  wire        clk,
    input  wire        rst_n,

    input  wire        op_valid,
    input  wire [33:0] op_data,
    output wire        op_room,

    output reg         cmd_valid,
    input  wire        cmd_ready,
    output wire        cmd_we,
    output reg  [23:0] cmd_addr,
    output reg  [3:0]  cmd_len,
    output reg         wr_valid,
    input  wire        wr_ready,
    output reg  [15:0] wr_data,
    output wire [1:0]  wr_be,

    output reg         pic_done
);
    assign cmd_we = 1'b1;
    assign wr_be  = 2'b11;

    // -----------------------------------------------------------------
    // The queue
    // -----------------------------------------------------------------
    reg [33:0] q [0:63];
    reg [5:0]  wp, rp;
    reg [6:0]  count;

    wire       q_empty = (count == 7'd0);
    wire [33:0] q_head = q[rp];
    reg        pop;

    // Eight free entries: a whole luma record and the deblocker's
    // registered push in flight.
    assign op_room = (count <= 7'd56);

    always @(posedge clk) begin
        if (op_valid) q[wp] <= op_data;
    end

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            wp <= 6'd0;
            rp <= 6'd0;
            count <= 7'd0;
        end else begin
            if (op_valid) wp <= wp + 6'd1;
            if (pop) rp <= rp + 6'd1;
            count <= count + {6'd0, op_valid} - {6'd0, pop};
        end
    end

    // -----------------------------------------------------------------
    // Bursts
    // -----------------------------------------------------------------
    localparam [2:0] W_HEAD = 3'd0;   // a header, or the end of the picture
    localparam [2:0] W_CMD  = 3'd1;   // the command waits for the port
    localparam [2:0] W_POP  = 3'd2;   // the next word, once it is queued
    localparam [2:0] W_LO   = 3'd3;   // its low half on the port
    localparam [2:0] W_HI   = 3'd4;   // its high half on the port

    reg [2:0]  ws;
    reg [2:0]  words;          // record words not yet taken from the queue
    reg [15:0] word_hi;

    wire [23:0] head_addr;
    wire [4:0]  head_run;
    frame_addr where (
        .frame({1'b0, q_head[31:28]}),
        .plane(q_head[27:26]),
        .x(q_head[14:4]),
        .y(q_head[25:15]),
        .addr(head_addr),
        .run(head_run)
    );
    // A row segment starts at a multiple of 16 (luma) or 8 (chroma)
    // samples, so `run` is always at least the 8 or 4 words of a burst.
    wire unused_run = &{1'b0, head_run};

    // The high beat accepted with another word waiting: take it at once.
    wire next_now = (ws == W_HI) && wr_ready && (words != 3'd0) && !q_empty;

    always @* begin
        case (ws)
            W_HEAD:  pop = !q_empty;
            W_POP:   pop = !q_empty;
            W_HI:    pop = next_now;
            default: pop = 1'b0;
        endcase
    end

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            ws <= W_HEAD; words <= 3'd0; word_hi <= 16'd0;
            cmd_valid <= 1'b0; cmd_addr <= 24'd0; cmd_len <= 4'd0;
            wr_valid <= 1'b0; wr_data <= 16'd0;
            pic_done <= 1'b0;
        end else begin
            pic_done <= 1'b0;
            case (ws)
                W_HEAD: if (!q_empty) begin
                    if (q_head[33:32] == 2'b10) begin
                        pic_done <= 1'b1;
                    end else if (q_head[33:32] == 2'b01) begin
                        cmd_valid <= 1'b1;
                        cmd_addr  <= head_addr;
                        cmd_len   <= {q_head[2:0], 1'b1};
                        words     <= q_head[2:0] + 3'd1;
                        ws        <= W_CMD;
                    end
                end
                W_CMD: if (cmd_ready) begin
                    cmd_valid <= 1'b0;
                    ws        <= W_POP;
                end
                W_POP: if (!q_empty) begin
                    wr_valid <= 1'b1;
                    wr_data  <= q_head[15:0];
                    word_hi  <= q_head[31:16];
                    words    <= words - 3'd1;
                    ws       <= W_LO;
                end
                W_LO: if (wr_ready) begin
                    wr_data <= word_hi;
                    ws      <= W_HI;
                end
                W_HI: if (wr_ready) begin
                    if (next_now) begin
                        wr_data <= q_head[15:0];
                        word_hi <= q_head[31:16];
                        words   <= words - 3'd1;
                        ws      <= W_LO;
                    end else begin
                        wr_valid <= 1'b0;
                        ws <= (words == 3'd0) ? W_HEAD : W_POP;
                    end
                end
                default: ws <= W_HEAD;
            endcase
        end
    end
endmodule
