module pmod_bidir (.sw0(sw0$pad), .sw1(sw1$pad), .jc1(jc1$pad), .led0(led0$pad), .led1(led1$pad));
  input sw0$pad;
  input sw1$pad;
  inout jc1$pad;
  output led0$pad;
  output led1$pad;
  wire led0;
  wire led1;
  wire sw0$pad;
  wire sw0$in0;
  wire sw1$pad;
  wire sw1$in0;
  wire jc1$drive;
  wire jc1$oe;
  wire jc1$pad;
  wire jc1$pin0;
  wire jc1$in0;
  wire led0$pad;
  wire led0$pin0;
  wire led1$pad;
  wire led1$pin0;
  assign jc1$pad = jc1$pin0;
  assign led0 = jc1$in0;
  assign led1 = sw0$in0;
  assign jc1$drive = sw1$in0;
  assign led0$pad = led0$pin0;
  assign led1$pad = led1$pin0;
  (* port = "sw0", pin = "V17", io_standard = "LVCMOS33" *)
  IBUF sw0$io0 (
    .I(sw0$pad),
    .O(sw0$in0)
  );
  (* port = "sw1", pin = "V16", io_standard = "LVCMOS33" *)
  IBUF sw1$io0 (
    .I(sw1$pad),
    .O(sw1$in0)
  );
  (* port = "jc1", pin = "H2", io_standard = "LVCMOS33", \pullup  = 1 *)
  IOBUF jc1$io0 (
    .I(jc1$drive),
    .T(jc1$oe),
    .IO(jc1$pin0),
    .O(jc1$in0)
  );
  (* port = "led0", pin = "U15", io_standard = "LVCMOS33" *)
  OBUF led0$io0 (
    .I(led0),
    .O(led0$pin0)
  );
  (* port = "led1", pin = "V14", io_standard = "LVCMOS33" *)
  OBUF led1$io0 (
    .I(led1),
    .O(led1$pin0)
  );
  LUT6 #(.INIT(64'h5555555555555555)) \$lut0  (
    .I0(sw0$in0),
    .I1(1'b0),
    .I2(1'b0),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(jc1$oe)
  );
endmodule
