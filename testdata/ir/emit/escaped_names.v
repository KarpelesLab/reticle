module Top (.clk(clk_i), \input , entity, \a_b , Foo, foo, \9lives , \reticle_x , _under, a$b, resize, \q[0] , mod);
  input clk_i;
  input \input ;
  input entity;
  input [1:0] \a_b ;
  input [1:0] Foo;
  output [1:0] foo;
  output \9lives ;
  output \reticle_x ;
  output _under;
  output a$b;
  output [3:0] resize;
  output \q[0] ;
  output mod;
  wire \input ;
  wire entity;
  wire [1:0] \a_b ;
  wire [1:0] Foo;
  wire [1:0] foo;
  wire \9lives ;
  wire \reticle_x ;
  wire _under;
  wire a$b;
  wire [3:0] resize;
  wire \q[0] ;
  wire clk_i;
  reg mod;
  assign foo = Foo & \a_b ;
  assign \9lives  = \input  | entity;
  assign \reticle_x  = \9lives ;
  assign _under = ~\input ;
  assign a$b = \input  ^ entity;
  assign resize = {{2{1'b0}}, Foo};
  assign \q[0]  = Foo[0];
  (* \ram_style  = "block" *)
  always @(posedge clk_i) begin : \always 
    mod <= entity;
  end
  (* keep = 1 *)
  \vendor_cell  #(.\INIT_VAL (4'h9)) \u.0  (
    .in(\input ),
    .\out_port (mod)
  );
endmodule
