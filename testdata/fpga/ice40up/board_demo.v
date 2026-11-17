// The two board designs in one image: the RGB LED stepping through its
// colours and the serial echo, side by side on the one 12 MHz clock. This
// is what an iCE40UP5K-SG48 board with an FT2232H was left booting from its
// flash, so that whoever picks it up next can see it work on both counts.
//
//   reticle fpga testdata/fpga/ice40up/board_demo.v \
//       testdata/fpga/ice40up/rgb_blink.v testdata/fpga/ice40up/uart_echo.v \
//       --device ice40-up5k-sg48 \
//       --constraints testdata/fpga/ice40up/board_demo.rcf \
//       --bitstream board_demo.bin
module board_demo (
    input  wire clk,
    input  wire rx,
    output wire tx,
    output wire led_r,
    output wire led_g,
    output wire led_b
);
    rgb_blink blink (.clk(clk), .led_r(led_r), .led_g(led_g), .led_b(led_b));
    uart_echo echo (.clk(clk), .rx(rx), .tx(tx));
endmodule
