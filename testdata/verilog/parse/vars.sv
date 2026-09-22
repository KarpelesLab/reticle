// Variable declarations: every keyword type, lifetimes, const, var,
// packed and unpacked dimensions, queues, associative arrays, events.
module vars;
    reg r;
    reg [7:0] r8, r8b = 8'h5a;
    reg signed [15:0] rs;
    logic l;
    logic [3:0] l4 = 4'b1010;
    logic [1:0][7:0] l2d;
    logic unsigned [7:0] lu;
    bit b;
    bit [63:0] b64;
    byte by = 8'd7;
    shortint si;
    int i = 42, j;
    longint li;
    integer n;
    time t;
    real re = 1.5e-3;
    realtime rt;
    shortreal sr;
    string s = "hello";
    chandle ch;
    event ev, ev2;
    genvar g;

    logic [7:0] mem [0:255];
    logic [7:0] mem2 [256];
    int dyn [];
    int q [$];
    int bounded_q [$:15];
    int assoc [string];
    int assoc_any [*];
    logic [7:0] mixed [2][3:0];

    var logic vl;
    var v_implicit;
    var [3:0] v_range;
    static int st = 0;
    automatic int au;
    const int K = 5;
    const logic [1:0] K2 = 2'b11;
    var int vi;
    static logic [1:0] sl;

    wire logic [7:0] wl;
    wire bit wb;
    var struct packed { logic a; logic [2:0] b; } sp;
    var enum { E0, E1 } en;
    struct { int x; int y; } point;
    union packed { logic [7:0] b; logic [1:0][3:0] n; } un;
    typedef int arr4_t [4];
    arr4_t a4 = '{1, 2, 3, 4};
    type(i) same_as_i;
    type(int) also_int;
endmodule
