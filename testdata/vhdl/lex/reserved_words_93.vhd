-- lex: vhdl93
-- In VHDL-93 mode the 2008 additions are ordinary identifiers, which real
-- legacy code relies on (`default` and `parameter` as signal names).
signal default : bit;
signal parameter, context, force, release : bit;
signal vunit, vmode, vprop, restrict, assume, cover : bit;
signal fairness, sequence, property, strong, protected : bit;
-- The 1993 words are still reserved.
ENTITY Architecture Process
