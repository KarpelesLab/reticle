// aac_pcm_out -- two frames of PCM, and the stream that plays them.
//
// The decoder writes a frame into one half while the other half plays, so
// one frame's synthesis overlaps the previous frame's output. A half holds
// 1024 samples of each channel; it is committed whole, with its channel
// count, and played interleaved: left, right, left, right. `out_last` is
// on a frame's final sample.
//
// out_valid comes from a two-word FIFO and depends on no input.

module aac_pcm_out (
    input  wire        clk,
    input  wire        rst_n,

    input  wire        pcm_we,
    input  wire        pcm_ch,
    input  wire [9:0]  pcm_n,
    input  wire [15:0] pcm_data,
    input  wire        commit,     // the half being written is complete
    input  wire        stereo,     // ... and holds two channels
    output wire        can_write,  // the half to write next is free

    output wire [15:0] out_data,
    output wire        out_right,
    output wire        out_last,
    output wire        out_valid,
    input  wire        out_ready
);
    reg  [1:0]  full;
    reg  [1:0]  two;
    reg         wr_half;
    reg         rd_half;

    wire [15:0] rdata, unused;
    aac_ram #(.WIDTH(16), .DEPTH(4096), .AW(12)) u_buf (
        .clk(clk), .we(pcm_we), .waddr({wr_half, pcm_ch, pcm_n}), .wdata(pcm_data),
        .raddr_a({rd_half, c, n}), .rdata_a(rdata), .raddr_b(12'd0), .rdata_b(unused)
    );

    assign can_write = !full[wr_half];

    // The reader: sample n of channel c of half rd_half, whose address is
    // in front of the RAM every cycle; an issue is a word out next cycle.
    reg  [9:0]  n;
    reg         c;
    reg         rv;        // a read is in flight
    reg         r_right, r_last;
    reg  [1:0]  cnt;       // FIFO words
    reg  [17:0] f0, f1;    // {last, right, data}

    wire pop   = out_ready && cnt != 2'd0;
    wire room  = ({1'b0, cnt} + {2'd0, rv}) < 3'd2 || (pop && ({1'b0, cnt} + {2'd0, rv}) < 3'd3);
    wire issue = full[rd_half] && room;
    wire last_issue = issue && n == 10'd1023 && (c || !two[rd_half]);

    assign out_valid = cnt != 2'd0;
    assign out_data  = f0[15:0];
    assign out_right = f0[16];
    assign out_last  = f0[17];

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            full <= 2'd0; two <= 2'd0; wr_half <= 1'b0; rd_half <= 1'b0;
            n <= 10'd0; c <= 1'b0; rv <= 1'b0;
            r_right <= 1'b0; r_last <= 1'b0; cnt <= 2'd0;
            f0 <= 18'd0; f1 <= 18'd0;
        end else begin
            if (commit) begin
                full[wr_half] <= 1'b1;
                two[wr_half] <= stereo;
                wr_half <= ~wr_half;
            end

            rv <= issue;
            if (issue) begin
                r_right <= c;
                r_last <= last_issue;
                if (c || !two[rd_half]) begin
                    c <= 1'b0;
                    n <= n + 10'd1;
                end else
                    c <= 1'b1;
                if (last_issue) begin
                    // The words are read out by the next edge, so the half
                    // may be written again from then on.
                    full[rd_half] <= 1'b0;
                    rd_half <= ~rd_half;
                    n <= 10'd0;
                    c <= 1'b0;
                end
            end

            // FIFO: f0 is the head.
            case ({rv, pop})
            2'b10: begin
                if (cnt == 2'd0) f0 <= {r_last, r_right, rdata};
                else f1 <= {r_last, r_right, rdata};
                cnt <= cnt + 2'd1;
            end
            2'b01: begin
                f0 <= f1;
                cnt <= cnt - 2'd1;
            end
            2'b11: begin
                if (cnt == 2'd1) f0 <= {r_last, r_right, rdata};
                else begin f0 <= f1; f1 <= {r_last, r_right, rdata}; end
            end
            default: ;
            endcase
        end
    end
endmodule
