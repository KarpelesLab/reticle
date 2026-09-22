// An interface bundle flattened into the modules that use it.
interface simple_if;
    logic [7:0] data;
    logic       valid;
    logic       ready;

    modport source (output data, output valid, input ready);
    modport sink   (input data, input valid, output ready);
endinterface

module producer (simple_if.source bus, input logic [7:0] d);
    assign bus.data  = d;
    assign bus.valid = 1'b1;
endmodule

module consumer (simple_if.sink bus, output logic [7:0] q);
    assign bus.ready = 1'b1;
    assign q         = bus.valid ? bus.data : 8'h00;
endmodule

module sv_interface(input logic [7:0] d, output logic [7:0] q);
    simple_if bus ();
    producer u_src  (.bus(bus), .d(d));
    consumer u_sink (.bus(bus), .q(q));
endmodule
