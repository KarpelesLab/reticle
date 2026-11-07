`timescale 1ns/1ps
module tb;
  string g;
  function automatic string greet(string who);
    return {"hi ", who};
  endfunction
  function automatic int slen(input string s);
    return s.len();
  endfunction
  task automatic show(input string tag, input int v);
    $display("SA %s=%0d", tag, v);
  endtask
  task automatic setstr(output string o);
    o = "set by task";
  endtask
  initial begin
    $display("SA1 %s", greet("bob"));
    $display("SA2 len=%0d", slen("abcdef"));
    show("x", 5);
    show("yy", -3);
    setstr(g);
    $display("SA3 g=%s", g);
    g = greet(g);
    $display("SA4 g=%s", g);
    $finish;
  end
endmodule
