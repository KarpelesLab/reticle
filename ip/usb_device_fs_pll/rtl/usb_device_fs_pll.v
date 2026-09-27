// usb_device_fs_pll — `usb_device_fs` with its 48 MHz from the device's PLL.
//
// What it does
//   Declares `clk48`, uses it, leaves it undriven and puts
//   `clock_mhz = 48` on it, which is how a design asks Reticle's FPGA
//   flow for a PLL (docs/fpga.md, "Generated clocks"). From the 12 MHz
//   oscillator most small boards carry, both families' PLLs make 48 MHz
//   exactly — on the iCE40 that is DIVR 0, DIVF 63, DIVQ 4 — so the
//   device's bit clock is as good as the board's crystal, well inside
//   the 0.25 % full speed allows. The board's constraints must say what
//   `clk_ref` is with a `create_clock`.
//
//   Every port of `usb_device_fs` is brought out, the data endpoint's byte
//   interface included. That is not decoration: a port a wrapper does not
//   pass on is an **input nothing drives**, and a flip-flop whose data
//   input nothing drives is what the ECP5 backend now refuses and what
//   cost the enumeration eight rounds to find. A wrapper that added a
//   clock and quietly dropped half a block's interface would be a trap.
//
// What it does not do
//   The PLL's lock output is not brought out by the flow, so hold
//   `rst_n` until the PLL has had time to lock. The wrapper cannot be
//   simulated, since nothing in the source drives its clock; simulate
//   `usb_device_fs` with a 48 MHz clock of its own, as the tests do.
module usb_device_fs_pll #(
    parameter [15:0]  VID          = 16'h1209,
    parameter [15:0]  PID          = 16'h0001,
    parameter [7:0]   DEV_CLASS    = 8'hFF,
    parameter [7:0]   DEV_SUBCLASS = 8'h00,
    parameter [7:0]   DEV_PROTOCOL = 8'h00,
    parameter [7:0]   CFG_ATTR     = 8'h80,
    parameter [7:0]   CFG_POWER    = 8'd50,
    // The class's interface and endpoint descriptors; `usb_ctrl_ep` says
    // how they are written and what is derived from them.
    parameter integer IFACE_BYTES  = 23,
    parameter [IFACE_BYTES*8-1:0] IFACE_DESC = {
        8'd9, 8'd4, 8'd0, 8'd0, 8'd2, 8'hFF, 8'h00, 8'h00, 8'd0,
        8'd7, 8'd5, 8'h01, 8'd2, 8'd8, 8'd0, 8'd0,
        8'd7, 8'd5, 8'h81, 8'd2, 8'd8, 8'd0, 8'd0
    },
    parameter [3:0]   DATA_ENDP    = 4'd1,
    parameter [3:0]   MAXPKT       = 4'd8
) (
    input  wire       clk_ref,
    input  wire       rst_n,
    input  wire       usb_dp_i,
    input  wire       usb_dn_i,
    output wire       usb_dp_o,
    output wire       usb_dn_o,
    output wire       usb_oe,
    output wire       usb_dp_pu,
    output wire [6:0] address,
    output wire       configured,
    output wire       usb_reset,

    // The data endpoint's bytes.
    output wire [7:0] out_data,
    output wire       out_valid,
    output wire       out_last,
    input  wire       out_ready,
    input  wire [7:0] in_data,
    input  wire       in_valid,
    output wire       in_ready,
    input  wire       in_commit
);
    (* clock_mhz = 48 *)
    wire clk48;

    usb_device_fs #(
        .VID          (VID),
        .PID          (PID),
        .DEV_CLASS    (DEV_CLASS),
        .DEV_SUBCLASS (DEV_SUBCLASS),
        .DEV_PROTOCOL (DEV_PROTOCOL),
        .CFG_ATTR     (CFG_ATTR),
        .CFG_POWER    (CFG_POWER),
        .IFACE_BYTES  (IFACE_BYTES),
        .IFACE_DESC   (IFACE_DESC),
        .DATA_ENDP    (DATA_ENDP),
        .MAXPKT       (MAXPKT)
    ) u_usb (
        .clk48      (clk48),
        .rst_n      (rst_n),
        .usb_dp_i   (usb_dp_i),
        .usb_dn_i   (usb_dn_i),
        .usb_dp_o   (usb_dp_o),
        .usb_dn_o   (usb_dn_o),
        .usb_oe     (usb_oe),
        .usb_dp_pu  (usb_dp_pu),
        .address    (address),
        .configured (configured),
        .usb_reset  (usb_reset),
        .out_data   (out_data),
        .out_valid  (out_valid),
        .out_last   (out_last),
        .out_ready  (out_ready),
        .in_data    (in_data),
        .in_valid   (in_valid),
        .in_ready   (in_ready),
        .in_commit  (in_commit)
    );
endmodule
