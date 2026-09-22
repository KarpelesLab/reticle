-- Every literal form, all well-formed.
-- Decimal integers, with underscores and exponents.
0 42 1_000_000 1E3 1e+3 12E0
-- Decimal reals.
3.14 0.5 1_0.0_1 1.0E-3 2.5e+2 6.02e23
-- Based literals: integer and real, with exponents and underscores.
2#1010# 8#777# 16#FF# 16#ff_ff# 10#42#E2 2#1.1# 16#F.8#e-1 16#DEAD_BEEF#
-- Physical literals are an abstract literal followed by a unit name.
10 ns 1.5 us
-- Character literals, including the delimiters themselves.
c := ('a', 'Z', '0', '1', ' ', ''', '"', '(', ')', '\', 'é');
-- String literals; doubled quotes, and the percent replacement.
"" "hello" "say ""hi""" %percent% %100%% sure% "x % y" %a "quoted" b%
-- Bit-string literals: bases, signedness, length prefixes, both delimiters.
b"1010" B"1_0" o"777" X"FF" x"f_f" ub"1" UO"7" ux"F" sb"1010" SO"7" sx"F"
d"255" 8x"F_F" 12UX"ABC" 16SB"1111" 4b%1010% x"ZZ" x"--"
