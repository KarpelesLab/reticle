// A register file on NewAE's USB bus, to test what a Husky Plus bitstream
// from this flow does on the part: the bus itself, carry chains (a 32-bit
// counter, a sum, a difference and a comparison the host checks against
// its own arithmetic), and the two LVDS clocks the board's PLL sends the
// FPGA, each counted in its own domain. `probe.py` says what each register
// is and checks it. The bus is NewAE's own `usb_reg_main.v`, from the
// chipwhisperer-husky-fpga checkout, so a fault found here is this flow's.
`default_nettype none
module husky_probe (
    input  wire       clk_usb,
    inout  wire [7:0] USB_Data,
    input  wire [7:0] USB_Addr,
    input  wire       USB_RDn,
    input  wire       USB_WRn,
    input  wire       USB_CEn,
    input  wire       USB_ALEn,
    input  wire       PLLFPGAP,
    input  wire       PLLFPGAN,
    input  wire       ADC_clk_fbp,
    input  wire       ADC_clk_fbn
);
    wire pll_pre, pll_clk, adc_pre, adc_clk;
    IBUFDS #(.DIFF_TERM("FALSE"), .IBUF_LOW_PWR("FALSE"), .IOSTANDARD("LVDS_25"))
        U_ibuf_pll (.I(PLLFPGAP), .IB(PLLFPGAN), .O(pll_pre));
    BUFG U_bufg_pll (.I(pll_pre), .O(pll_clk));
    IBUFDS #(.DIFF_TERM("FALSE"), .IBUF_LOW_PWR("FALSE"), .IOSTANDARD("LVDS_25"))
        U_ibuf_adc (.I(ADC_clk_fbp), .IB(ADC_clk_fbn), .O(adc_pre));
    BUFG U_bufg_adc (.I(adc_pre), .O(adc_clk));
    reg [31:0] pll_count = 32'd0, adc_count = 32'd0;
    always @(posedge pll_clk) pll_count <= pll_count + 32'd1;
    always @(posedge adc_clk) adc_count <= adc_count + 32'd1;
    wire [7:0] reg_address;
    wire [6:0] reg_bytecnt;
    wire [7:0] write_data;
    reg  [7:0] read_data;
    wire       reg_read, reg_write, isout;
    wire [7:0] dout;
    wire [7:0] din = USB_Data;
    usb_reg_main #(.pBYTECNT_SIZE(7)) U_usb_reg_main (
        .clk_usb(clk_usb), .reset(1'b0),
        .cwusb_din(din), .cwusb_dout(dout), .cwusb_isout(isout),
        .cwusb_addr(USB_Addr), .cwusb_rdn(USB_RDn), .cwusb_wrn(USB_WRn),
        .cwusb_cen(USB_CEn), .cwusb_alen(USB_ALEn), .fast_fifo_read(1'b0),
        .reg_address(reg_address), .reg_bytecnt(reg_bytecnt),
        .reg_datao(write_data), .reg_datai(read_data),
        .reg_read(reg_read), .reg_write(reg_write), .debug()
    );
    assign USB_Data = isout ? dout : 8'bz;

    wire [1:0] lane = reg_bytecnt[1:0];
    reg [7:0]  echo = 8'd0;
    reg [31:0] a = 32'd0, b = 32'd0, snap = 32'd0;
    reg [31:0] counter = 32'd0;
    reg [31:0] sum_r = 32'd0, diff_r = 32'd0;
    reg        lt_r = 1'b0;
    reg [7:0]  count8 = 8'd0;
    always @(posedge clk_usb) begin
        counter <= counter + 32'd1;
        sum_r   <= a + b;
        diff_r  <= a - b;
        lt_r    <= a < b;
        count8   <= count8 + 8'd1;
    end
    always @(posedge clk_usb) begin
        if (reg_write) begin
            case (reg_address)
                8'd4:  echo <= write_data;
                8'd60: a[lane*8 +: 8] <= write_data;
                8'd61: b[lane*8 +: 8] <= write_data;
                8'd63: snap <= counter;
                default: ;
            endcase
        end
    end
    always @(*) begin
        case (reg_address)
            8'd4:  read_data = echo;
            8'd10: read_data = (lane == 2'd0) ? 8'd2 : (lane == 2'd1) ? 8'd88 : 8'd0;
            8'd60: read_data = a[lane*8 +: 8];
            8'd61: read_data = b[lane*8 +: 8];
            8'd62: read_data = sum_r[lane*8 +: 8];
            8'd63: read_data = snap[lane*8 +: 8];
            8'd64: read_data = diff_r[lane*8 +: 8];
            8'd65: read_data = {7'd0, lt_r};
            8'd66: read_data = counter[lane*8 +: 8];
            8'd67: read_data = count8;
            8'd70: read_data = pll_count[lane*8 +: 8];
            8'd71: read_data = adc_count[lane*8 +: 8];
            default: read_data = 8'd0;
        endcase
    end
endmodule
