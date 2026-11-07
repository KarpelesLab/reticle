`timescale 1ns/1ps
module tb;
  string s, t, u;
  int n;
  initial begin
    $display("ST1 empty len=%0d '%s'", s.len(), s);
    s = "Hello";
    t = "World";
    $display("ST2 s=%s len=%0d", s, s.len());
    u = {s, ", ", t};
    $display("ST3 u=%s len=%0d", u, u.len());
    $display("ST4 cmp eq=%b ne=%b lt=%b gt=%b", s == t, s != t, s < t, s > t);
    $display("ST5 compare=%0d icompare=%0d", s.compare(t) < 0, s.icompare("HELLO"));
    $display("ST6 substr=%s", u.substr(7, 11));
    $display("ST7 upper=%s lower=%s", s.toupper(), s.tolower());
    $display("ST8 getc=%0d %c", s.getc(1), s.getc(1));
    s.putc(0, "J");
    $display("ST9 putc s=%s", s);
    s = "1234";
    $display("ST10 atoi=%0d", s.atoi());
    s = "ff";
    $display("ST11 atohex=%0d", s.atohex());
    s.itoa(-56);
    $display("ST12 itoa=%s", s);
    s.hextoa(255);
    $display("ST13 hextoa=%s", s);
    u = $sformatf("v=%0d h=%h b=%b", 10, 8'hAB, 3'b101);
    $display("ST14 sformatf=%s", u);
    n = 3;
    u = {n{"ab"}};
    $display("ST15 rep=%s", u);
    s = "abc";
    $display("ST16 s[1]=%c", s[1]);
    $display("ST17 string literal to bits %h", "AB");
    u = "";
    $display("ST18 empty eq=%b", u == "");
    $finish;
  end
endmodule
