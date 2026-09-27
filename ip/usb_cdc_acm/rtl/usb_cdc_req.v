// usb_cdc_req — the three class requests a CDC ACM serial port answers, on
// `usb_ctrl_ep`'s class hook.
//
// What it does
//   Endpoint 0 in `ip/usb_device_fs` answers the standard requests and
//   offers everything else to whatever is above it. This is what a CDC ACM
//   device puts there: the requests a host's own serial driver sends, and
//   nothing else.
//
//   Three of them, and each is a row of a published table:
//
//     SET_LINE_CODING         21h 20h   PSTN 1.2 Table 13, §6.3.10
//     GET_LINE_CODING         A1h 21h   PSTN 1.2 Table 13, §6.3.11
//     SET_CONTROL_LINE_STATE  21h 22h   PSTN 1.2 Table 13, §6.3.12
//
//   `21h` is host-to-device, class, to an interface, and `A1h` is the same
//   the other way; USB 2.0 Table 9-2 is where those bits are. `wIndex` is
//   the interface the request is for, and this block answers only for the
//   **communications** interface, which `usb_cdc_acm`'s descriptors number
//   `COMM_IFACE` — 0. A request to any other interface is not claimed and
//   endpoint 0 stalls it, which is what a host asking the wrong interface
//   should get.
//
//   The line coding is seven bytes and this block keeps them: dwDTERate,
//   bCharFormat, bParityType and bDataBits, in that order (PSTN 1.2 Table
//   17). SET_LINE_CODING replaces them, GET_LINE_CODING reads them back,
//   and they come out of this module as `baud`, `char_format`, `parity`
//   and `data_bits` for whatever is above. **Nothing here acts on them**:
//   a serial port whose divisor followed dwDTERate would need a divide by
//   a run-time value, which is a design's business and not this block's.
//   They are reported so that a design *can*.
//
//   SET_CONTROL_LINE_STATE carries two bits in `wValue` — D0 is DTR and D1
//   is RTS (PSTN 1.2 §6.3.12) — and they come out as `dtr` and `rts`. A
//   host's driver raises DTR when a program opens the port and drops it
//   when the last one closes, so `dtr` is the one signal on this block
//   that says *somebody is listening*.
//
//   The timing is `usb_ctrl_ep`'s and is stated there: `claim`, `len` and
//   `resp` are combinational in `setup`, read in the one cycle `req` is
//   high. That is why the decoding below is `assign` and not a state
//   machine — a request decoder is a comparison of eight bytes against
//   constants, and there is nothing in it to sequence.
//
// What it does not do
//   No SEND_BREAK, and **that is stated in the descriptor rather than only
//   here**: the Abstract Control Management functional descriptor
//   `usb_cdc_acm` writes has bmCapabilities `02h`, which is D1 alone, and
//   PSTN 1.2 Table 4 makes D2 the bit that claims SEND_BREAK. A host that
//   reads that descriptor does not send one; one that sends it anyway is
//   not claimed here and endpoint 0 stalls it, which is the right answer
//   for a request the device never offered.
//
//   No SET_COMM_FEATURE, GET_COMM_FEATURE or CLEAR_COMM_FEATURE (D0 of the
//   same byte, also clear), and no network-connection notification (D3).
//
//   No **SERIAL_STATE** notification, which is the one omission worth
//   reading twice. It is not a request at all — it is a ten-byte interrupt
//   IN packet — and `usb_bulk_ep` holds eight. `usb_cdc_acm`'s header says
//   what that costs and what established that Linux does not need one.
//
//   Nothing here touches the bulk endpoints. The serial port's bytes go
//   through `usb_bulk_ep`'s byte interface and never come near endpoint 0.
module usb_cdc_req #(
    // The communications interface's `bInterfaceNumber`, which is the
    // `wIndex` these requests carry. It is a parameter because it has to
    // agree with the descriptors, and `usb_cdc_acm` passes its own.
    parameter [7:0] COMM_IFACE = 8'd0,
    // What the line coding is before a host has said otherwise: 9600 8N1,
    // which is what a serial port with no opinion has been since long
    // before USB. bCharFormat 0 is one stop bit and bParityType 0 is none
    // (PSTN 1.2 Table 17).
    parameter [31:0] RATE_RESET   = 32'd9600,
    parameter [7:0]  FORMAT_RESET = 8'd0,
    parameter [7:0]  PARITY_RESET = 8'd0,
    parameter [7:0]  BITS_RESET   = 8'd8
) (
    input  wire        clk,
    input  wire        rst_n,

    // `usb_ctrl_ep`'s class hook.
    input  wire [63:0] setup,
    input  wire        req,
    output wire        claim,
    output wire [6:0]  len,
    input  wire [6:0]  index,
    output wire [7:0]  resp,
    input  wire [63:0] out_data,
    input  wire [3:0]  out_len,
    input  wire        out_valid,

    // What the host asked the line to be.
    output wire [31:0] baud,
    output wire [7:0]  char_format,
    output wire [7:0]  parity,
    output wire [7:0]  data_bits,
    // The control lines it asserted. `dtr` is a program having opened the
    // port.
    output wire        dtr,
    output wire        rts
);
    // The SETUP packet's fields, byte 0 in the low eight bits (USB 2.0
    // Table 9-2).
    wire [7:0]  bm_request_type = setup[7:0];
    wire [7:0]  b_request       = setup[15:8];
    wire [15:0] w_value         = setup[31:16];
    wire [15:0] w_index         = setup[47:32];
    wire [15:0] w_length        = setup[63:48];

    // bRequest, from PSTN 1.2 Table 13.
    localparam [7:0] REQ_SET_LINE_CODING        = 8'h20;
    localparam [7:0] REQ_GET_LINE_CODING        = 8'h21;
    localparam [7:0] REQ_SET_CONTROL_LINE_STATE = 8'h22;

    // bmRequestType: class, to an interface, in each direction.
    localparam [7:0] TYPE_OUT = 8'h21;
    localparam [7:0] TYPE_IN  = 8'hA1;

    // The interface these requests are for. Only the low byte of `wIndex`
    // is an interface number; the high byte is reserved and zero.
    wire to_comm = (w_index == {8'h00, COMM_IFACE});

    wire set_line = (bm_request_type == TYPE_OUT)
                  & (b_request == REQ_SET_LINE_CODING)
                  & to_comm
                  & (w_length == 16'd7);
    wire get_line = (bm_request_type == TYPE_IN)
                  & (b_request == REQ_GET_LINE_CODING)
                  & to_comm;
    wire set_ctrl = (bm_request_type == TYPE_OUT)
                  & (b_request == REQ_SET_CONTROL_LINE_STATE)
                  & to_comm
                  & (w_length == 16'd0);

    assign claim = set_line | get_line | set_ctrl;
    // The line coding structure's length. Endpoint 0 caps it at the host's
    // own `wLength`, so a host asking for fewer than seven bytes gets what
    // it asked for.
    assign len   = 7'd7;

    // -----------------------------------------------------------------
    // The line coding, and the control lines.
    // -----------------------------------------------------------------
    reg [31:0] rate;
    reg [7:0]  format_q;
    reg [7:0]  parity_q;
    reg [7:0]  bits_q;
    reg        dtr_q;
    reg        rts_q;

    assign baud        = rate;
    assign char_format = format_q;
    assign parity      = parity_q;
    assign data_bits   = bits_q;
    assign dtr         = dtr_q;
    assign rts         = rts_q;

    // GET_LINE_CODING, a byte at a time, in the structure's own order
    // (PSTN 1.2 Table 17). Seven bytes, so the index needs three bits and
    // the eighth value cannot be asked for — `len` is 7 and endpoint 0
    // fetches `0` to `len - 1` — but the `case` is full anyway, because a
    // `case` that is not is a latch.
    reg [7:0] byte_q;
    always @(*) begin
        case (index[2:0])
            3'd0:    byte_q = rate[7:0];
            3'd1:    byte_q = rate[15:8];
            3'd2:    byte_q = rate[23:16];
            3'd3:    byte_q = rate[31:24];
            3'd4:    byte_q = format_q;
            3'd5:    byte_q = parity_q;
            default: byte_q = bits_q;
        endcase
    end
    assign resp = byte_q;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            rate     <= RATE_RESET;
            format_q <= FORMAT_RESET;
            parity_q <= PARITY_RESET;
            bits_q   <= BITS_RESET;
            dtr_q    <= 1'b0;
            rts_q    <= 1'b0;
        end else begin
            // SET_CONTROL_LINE_STATE has no data stage, so `wValue` in the
            // cycle the request arrives is the whole of it.
            if (req && set_ctrl) begin
                dtr_q <= w_value[0];
                rts_q <= w_value[1];
            end
            // SET_LINE_CODING's seven bytes arrive as the data stage's one
            // packet. `out_len` is checked because this block claims two
            // requests that write and only one of them carries data: a
            // SET_CONTROL_LINE_STATE that somehow brought a payload must
            // not land in the line coding.
            if (out_valid && out_len == 4'd7) begin
                rate     <= out_data[31:0];
                format_q <= out_data[39:32];
                parity_q <= out_data[47:40];
                bits_q   <= out_data[55:48];
            end
        end
    end
endmodule
