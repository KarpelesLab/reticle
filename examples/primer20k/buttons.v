// Four of the Tang Primer 20K dock's buttons, each lighting one LED while
// it is held. Written to find out which button is which: the dock's
// silkscreen follows neither its schematic's net numbers nor Apicula's
// board file, so only pressing them says.
//
// A dock button pulls its pin low when pressed, and a dock LED lights when
// its pin is driven low, so each LED is simply its button: idle, the pin
// reads high and the LED stays dark; held, both go low and it lights.
//
// The fifth button, on ball T10, is not here: T10 is also the part's SSPI
// `SI` configuration pin, which `reticle fpga` will not use as IO without
// a dual-purpose setting it does not write. Of the dock's S0 to S3, the
// one that lights nothing is therefore T10's.
//
// WHAT A PERSON SHOULD SEE: holding S4 lights one LED (the one on L16) —
// the control, since S4 is already known to be ball T3 — and holding
// three of S0 to S3 lights one other LED each.
module buttons (
    input  wire [3:0] key,
    output wire [3:0] led
);
    assign led = key;
endmodule
