module \escaped-module ;
    wire \bus[0] , \bus[1] ;
    wire \a+b = \bus[0] & \bus[1] ;
    // An escaped keyword is an identifier.
    reg \module ;
    reg \logic	;
    // Escaped identifiers end at any whitespace, including a newline.
    wire \last
    ;
    assign \a+b = \module;
endmodule
