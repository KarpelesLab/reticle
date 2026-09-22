// SystemVerilog expressions: casts, inside, streaming, assignment
// patterns, type arguments, scoped names, unsized literals, queues, new,
// wildcard equality, string methods and increment forms.
module exprs_sv;
    logic [7:0] a, b;
    logic [31:0] w;
    int i, j;
    int q [$];
    int dyn [];
    string s;
    typedef logic [15:0] half_t;
    half_t h;
    logic [3:0] nib;

    // Casts.
    assign w = int'(a);
    assign w = 16'(a);
    assign w = (i + 1)'(a);
    assign w = signed'(a);
    assign w = unsigned'(a);
    assign w = const'(a);
    assign w = half_t'(a);
    assign w = pkg::t'(a);
    assign w = logic'(a);
    assign w = string'(a);
    assign w = half_t'{default: 0};
    assign w = 8'(a) + 8'(b);

    // inside and wildcard comparison.
    assign nib[0] = a inside {8'd1, 8'd2, [8'd10:8'd20]};
    assign nib[1] = (a inside {b, [0:3]}) && b[0];
    assign nib[2] = a ==? 8'b1x0z_????;
    assign nib[3] = a !=? b;

    // Streaming and patterns.
    assign w = {<< {a, b}};
    assign w = {>> 8 {a, b}};
    assign w = {<< byte {w}};
    assign w = '{1, 2, 3, 4};
    assign w = '{default: '0, 1: 8'd5};
    assign w = '{int: 0, logic: 1};
    assign w = '{4{8'd0}};
    assign w = '{};

    // Unsized literals, $ and type arguments.
    assign a = '0;
    assign a = '1;
    assign a = 'x;
    assign a = 'z;
    assign i = $bits(int);
    assign i = $bits(logic [3:0]);
    assign i = $bits(half_t);
    assign i = $bits(a);
    assign i = $size(dyn, 1);
    assign i = $clog2(pkg::DEPTH);
    assign i = pkg::f(a);
    assign i = pkg::CONST + $unit::G;

    initial begin
        q.push_back(1);
        i = q.size();
        i = q[$];
        i = q[$-1];
        q = q[1:$];
        dyn = new[8];
        dyn = new[16](dyn);
        s = "abc";
        i = s.len();
        i = s.substr(0, 1).len();
        i = a.b.c.d;
        j = i++ + ++i;
        j = i-- - --i;
        w = a ** 2 - -b;
        w = (a = b);
        w = 10ns;
        w = 1step;
        w = 1.5ms;
        i = null == null;
        i = $root.top.i;
        w = a.first();
        w = arr.sum;
        w = {>>{a}};
    end
endmodule
