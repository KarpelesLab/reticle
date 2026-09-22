module strings;
    initial begin
        $display("plain");
        $display("tab\there, newline\nthere");
        $display("quote \" and backslash \\");
        $display("octal \101\102 hex \x43 bell \a ff \f vt \v");
        $display("percent %d %h %%", a, b);
        $display("continued \
across lines");
        $display("");
        $write("no newline");
        $sformatf("%0d", "`not_a_macro");
    end
endmodule
