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
// What it does not do
//   The PLL's lock output is not brought out by the flow, so hold
//   `rst_n` until the PLL has had time to lock. The wrapper cannot be
//   simulated, since nothing in the source drives its clock; simulate
//   `usb_device_fs` with a 48 MHz clock of its own, as the tests do.
module usb_device_fs_pll #(
    parameter [15:0] VID = 16'h1209,
    parameter [15:0] PID = 16'h0001
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
    output wire       usb_reset
);
    (* clock_mhz = 48 *)
    wire clk48;

    usb_device_fs #(.VID(VID), .PID(PID)) u_usb (
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
        .usb_reset  (usb_reset)
    );
endmodule
