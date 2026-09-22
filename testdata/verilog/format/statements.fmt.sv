// Statement forms: assignments of every operator, blocks, labels, if
// variants, procedural assign/force, triggers, disable, return/break,
// declarations at block start, and expression statements.
module stmts;
  logic [7:0] a, b, c;
  int i, j;
  event ev;
  logic [3:0] arr[4];

  initial begin : main
    a = 8'd1;
    b <= a;
    a += 1;
    a -= 1;
    a *= 2;
    a /= 2;
    a %= 3;
    a &= 8'hf0;
    a |= 8'h0f;
    a ^= 8'hff;
    a <<= 1;
    a >>= 1;
    a <<<= 1;
    a >>>= 1;
    i++;
    i--;
    ++i;
    --i;
    {a, b}    = {b, a};
    a[3:0]    = b[7:4];
    a[i]      = b[j];
    a[i+:2]   = b[j-:2];
    arr[1][2] = 1'b1;
    ;
  end : main

  initial begin
    int local_i = 3;
    logic [1:0] local_l;
    automatic int auto_i = 4;
    static int static_i = 5;
    localparam int LP = 6;
    parameter P = 7;
    typedef logic [1:0] two_t;
    two_t tt;
    begin : inner
      int deep = LP + P;
      tt = deep[1:0];
    end : inner
  end

  always @(a) begin
    if (a == 0) b = 1;
    if (a == 1) b = 2;
    else b = 3;
    if (a == 2) begin
      b = 4;
    end else if (a == 3) begin
      b = 5;
    end else begin
      b = 6;
    end
    unique if (a[0]) b = 7;
    else if (a[1]) b = 8;
    unique0 if (a[0]) b = 9;
    priority if (a[0]) b = 10;
    else b = 11;
    if (a) if (b) c = 1;
    else c = 2;
  end

  initial begin
    assign a = b;
    deassign a;
    force c = 8'hff;
    release c;
    -> ev;
    ->> ev;
    disable main;
    disable fork;
    wait fork;
    $display("done");
    $finish;
    $finish();
    void'(some_func(a));
    some_task(a, b);
    top.sub.some_task;
    lbl : a = 1;
    lbl2 : begin
      b = 2;
    end
    fork
      a = 1;
      b = 2;
    join
    fork : named_fork
      #1 a = 1;
      #2 b = 2;
    join_any : named_fork
    fork
      c = 3;
    join_none
    (* full_case *) a = 1;
  end

  function int f(int x);
    if (x < 0) return -x;
    return x;
  endfunction

  task t;
    int k;
    for (k = 0; k < 4; k++) begin
      if (k == 2) continue;
      if (k == 3) break;
    end
    return;
  endtask
endmodule
