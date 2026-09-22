-- Error recovery: every problem yields one diagnostic and lexing goes on.
a <= "unterminated string
b <= %also unterminated
c <= \unterminated extended
d <= \\;
e <= 17#10#;
f <= 8#79#;
g <= 16#FF;
h <= 1__0 + 1_ + 16#_F# + 2#1_#;
i <= 1E-3;
j <= 2#101#E-1;
k <= 16##;
l <= bad__name + trailing_;
m <= a $ b;
n <= '日';
o <= x"unterminated bit string
p <= a € b;
q <= 12ux"F" ;
r <= 16#F.#;
s <= a ~ b;
t <= x"1"; -- fine again
/* unterminated delimited comment, must be last
u <= 1;
