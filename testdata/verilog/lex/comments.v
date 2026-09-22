// Comment forms. A `define inside a comment is not a directive.
/* block */ module c; // line
    /* a // line comment inside a block comment is just text */
    // a /* block opener inside a line comment is just text
    wire /* inline */ a /**/ ; //
    /*
     * multi-line
     */
    wire b; /* one */ /* two */
endmodule
/* unterminated block comment swallows the rest
wire never_seen;
