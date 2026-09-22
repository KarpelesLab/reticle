-- Apostrophe disambiguation: character literal versus attribute tick.
v := x'(1);
s := t'image(v);
q <= '0';
w <= ('0', '1');
if sig'event and sig = '1' then
y <= std_logic'('1');
z := \ext\'('a');
n := a(1)'length;
m := "abc"'length;
p := character'val(65);
r := x'range;
c := '(';
d := ')';
e := (''', '"');
f := t'('(');
