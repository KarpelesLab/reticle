module mux4 (
  input wire [1:0] sel,
  input wire [7:0] a,
  input wire [7:0] b,
  input wire [7:0] c,
  input wire [7:0] d,
  output wire [7:0] y,
  output reg [7:0] y2
);
  wire [7:0] lo;
  wire [7:0] hi;
  assign lo = sel[0] ? b : a;
  assign hi = sel[1] ? d : c;
  assign y = sel[1] ? hi : lo;
  always @* begin : decode
    (* parallel_case, full_case *)
    case (sel)
      2'h0: begin
        y2 = a;
      end
      2'h1: begin
        y2 = b;
      end
      2'h2, 2'h3: begin
        y2 = sel[0] ? d : c;
      end
    endcase
  end
endmodule
