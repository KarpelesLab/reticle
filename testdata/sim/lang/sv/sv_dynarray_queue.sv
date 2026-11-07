`timescale 1ns/1ps
module tb;
  int d [];
  int q [$];
  int x;
  initial begin
    d = new[4];
    $display("D1 size=%0d d0=%0d", d.size(), d[0]);
    foreach (d[i]) d[i] = i + 100;
    $display("D2 d=%0d %0d %0d %0d", d[0], d[1], d[2], d[3]);
    d = new[6](d);
    $display("D3 size=%0d d3=%0d d5=%0d", d.size(), d[3], d[5]);
    d.delete();
    $display("D4 size=%0d", d.size());
    q.push_back(1);
    q.push_back(2);
    q.push_front(0);
    $display("D5 q size=%0d q0=%0d q2=%0d", q.size(), q[0], q[2]);
    x = q.pop_front();
    $display("D6 pop_front=%0d size=%0d", x, q.size());
    x = q.pop_back();
    $display("D7 pop_back=%0d size=%0d", x, q.size());
    q.insert(0, 42);
    $display("D8 q=%p", q);
    q = {q, 7, 8};
    $display("D9 q=%p $=%0d", q, q[$]);
    $finish;
  end
endmodule
