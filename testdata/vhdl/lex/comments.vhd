-- A line comment at the very start.
entity c is -- trailing comment with Unicode: é ü 日本語 🙂
end entity; --no space after the dashes
--
-- Empty comment above; a comment containing "quotes" and 'ticks' below.
-- "unterminated string inside a comment is fine 'x
a <= b; ---- extra dashes
-- Tool directives are recorded, skipped, and noted.
`protect begin_protected
`protect key_keyowner = "Reticle", key_method = "rsa"
`protect end_protected
`
x <= y;
