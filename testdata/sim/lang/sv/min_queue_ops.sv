// Queues and dynamic arrays past their elements: what an element they do
// not have reads, `delete(i)` and `insert` in the middle, popping an empty
// queue, `foreach`, assigning one from another and from a pattern, and
// `new[n](d)` from another dynamic array.
`timescale 1ns/1ps
module tb;
  int q [$];
  int r [$];
  logic [3:0] lq [$];
  int d [];
  int e [];
  int x, sum;
  initial begin
    q = '{5, 6, 7, 8};
    $display("Q1 size=%0d q=%p", q.size(), q);
    q.delete(1);
    $display("Q2 delete(1) q=%p", q);
    q.insert(2, 99);
    $display("Q3 insert(2) q=%p last=%0d", q, q[$]);
    $display("Q4 beyond=%0d", q[10]);
    q.delete();
    x = q.pop_front();
    $display("Q5 empty pop=%0d size=%0d", x, q.size());
    q.push_front(1);
    q.push_back(2);
    r = q;
    r.push_back(3);
    $display("Q6 q=%p r=%p", q, r);
    sum = 0;
    foreach (r[i]) sum += r[i] * (i + 1);
    $display("Q7 sum=%0d", sum);
    lq.push_back(4'hA);
    $display("Q8 lq=%p beyond=%b", lq, lq[3]);
    d = new[3];
    foreach (d[i]) d[i] = 10 * (i + 1);
    e = new[5](d);
    $display("Q9 e=%p size=%0d", e, e.size());
    r = {q, r};
    $display("Q10 r=%p", r);
    $finish;
  end
endmodule
