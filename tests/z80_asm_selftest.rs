//! The Z80 assembler in `tests/z80_asm/mod.rs`, checked against itself.
//!
//! The assembler exists so that a test can state a Z80 program in
//! assembly and know, independently of the core under test, which bytes
//! that program is. Nothing here involves a core: these tests ask whether
//! the table is self-consistent and whether the assembler emits what the
//! table says, which is the most that can be asked without a second
//! source of truth. They cannot tell whether an opcode byte or a T-state
//! count is the one Zilog published — that was typed out from the
//! reference, and only reading the reference again, or running the
//! program on a part, can confirm it. What they *can* catch is the class
//! of mistake that typing a 698-entry table invites: two instructions
//! given the same opcode, an operand shape no source text can reach, an
//! encoding whose length does not match the bytes the assembler writes
//! for it, and a displacement or prefix put in the wrong place.

#[path = "z80_asm/mod.rs"]
mod z80;

use std::collections::BTreeMap;

/// `ld hl,(nn)` and `ld (nn),hl` each have two documented encodings, and
/// the short one wins; see rule 6 of the assembler's module
/// documentation. These are the `ed` ones that are therefore unreachable
/// from source text, and the only two entries allowed to be.
const SHADOWED: &[(&str, &str, &[u8])] = &[
    ("ld", "(nn),hl", &[0xED, 0x63]),
    ("ld", "hl,(nn)", &[0xED, 0x6B]),
];

/// Whether an entry is one of [`SHADOWED`].
fn is_shadowed(insn: &z80::Insn) -> bool {
    SHADOWED
        .iter()
        .any(|(name, shape, code)| *name == insn.name && *shape == insn.shape && *code == insn.code)
}

/// A line of source text that writes `shape`, with each literal field
/// filled in by a value this file can recognise again.
///
/// A `label` shape gets a label named `after`, which the caller is
/// expected to place immediately after the instruction so that the
/// displacement is zero.
fn source_for(insn: &z80::Insn) -> String {
    let operands: Vec<String> = z80::tokens(insn.shape)
        .map(|token| match token {
            "n" => "$12".to_owned(),
            "nn" => "$1234".to_owned(),
            "(n)" => "($fe)".to_owned(),
            "(nn)" => "($1234)".to_owned(),
            "(ix+d)" => "(ix+5)".to_owned(),
            "(iy+d)" => "(iy-2)".to_owned(),
            "label" => "after".to_owned(),
            fixed => fixed.to_owned(),
        })
        .collect();
    if operands.is_empty() {
        insn.name.to_owned()
    } else {
        format!("{} {}", insn.name, operands.join(","))
    }
}

/// The bytes [`source_for`] must assemble to, built from the entry's own
/// `code` and `Field` rather than from the assembler's encoder, so that
/// the two have to agree.
fn expected_for(insn: &z80::Insn) -> Vec<u8> {
    let indexed_iy = z80::tokens(insn.shape).any(|token| token == "(iy+d)");
    let disp = if indexed_iy { 0xFE } else { 0x05 };
    let mut out = insn.code.to_vec();
    // A `dd cb` bit operation carries its displacement between the `cb`
    // and the opcode byte; everything else carries its literals last.
    let dd_cb = out.len() == 3 && out[1] == 0xCB;
    match insn.field() {
        z80::Field::None => {}
        z80::Field::Byte => out.push(0x12),
        z80::Field::Port => out.push(0xFE),
        z80::Field::Word => out.extend_from_slice(&[0x34, 0x12]),
        z80::Field::Rel => out.push(0x00),
        z80::Field::Disp if dd_cb => out.insert(2, disp),
        z80::Field::Disp => out.push(disp),
        z80::Field::DispByte => out.extend_from_slice(&[disp, 0x12]),
    }
    out
}

/// Every entry's `code` is a plausible prefix-plus-opcode sequence, and
/// its `Field` agrees with the shape it was written with.
///
/// This would catch a code slice with a prefix byte in the wrong place —
/// `&[0x06, 0xCB]` for a `cb` instruction, say — or a shape whose literal
/// field is not the one its bytes leave room for. It would not catch a
/// wrong opcode byte, since nothing here knows what the right one is.
#[test]
fn every_encoding_is_a_prefix_and_an_opcode() {
    for insn in z80::TABLE {
        let code = insn.code;
        let shape = insn.shape;
        let name = insn.name;
        match code {
            [_] => {}
            [0xCB | 0xED | 0xDD | 0xFD, _] => {}
            [0xDD | 0xFD, 0xCB, _] => assert_eq!(
                insn.field(),
                z80::Field::Disp,
                "`{name} {shape}` is a `dd cb` encoding, which is always `(ix+d)` or `(iy+d)`"
            ),
            other => panic!("`{name} {shape}` has the unexpected encoding {other:02x?}"),
        }
        assert_eq!(
            usize::from(insn.len()),
            code.len() + usize::from(insn.field().operand_bytes()),
            "`{name} {shape}` disagrees with itself about its length"
        );
        // A prefixed `cb` form is an index form and nothing else: the
        // unprefixed `cb` space never carries a literal.
        if code.first() == Some(&0xCB) {
            assert_eq!(
                insn.field(),
                z80::Field::None,
                "`{name} {shape}` is in the `cb` space, which has no literals"
            );
        }
    }
}

/// No two entries share their fixed bytes.
///
/// This is the test that would have caught a mistyped opcode byte in the
/// mechanical parts of the table — the 248 `cb` forms, the 62 `dd cb`
/// ones, `ld r,r'`, the arithmetic group — because a digit in the wrong
/// place there almost always lands on a neighbour. It would not catch two
/// instructions swapped with each other, nor an opcode mistyped into one
/// of the eight holes the omitted `sll` leaves.
#[test]
fn no_two_encodings_share_their_opcode_bytes() {
    let mut seen: BTreeMap<&[u8], String> = BTreeMap::new();
    for insn in z80::TABLE {
        if let Some(first) = seen.insert(insn.code, insn.syntax()) {
            panic!(
                "{:02x?} is both `{first}` and `{}`",
                insn.code,
                insn.syntax()
            );
        }
    }
    assert_eq!(seen.len(), z80::TABLE.len());
    assert_eq!(z80::TABLE.len(), 698, "the table changed size");
    // And the count of distinct operand shapes the module
    // documentation gives for `Field`.
    let mut shapes: Vec<&str> = z80::TABLE.iter().map(|insn| insn.shape).collect();
    shapes.sort_unstable();
    shapes.dedup();
    assert_eq!(shapes.len(), 298, "the distinct operand shapes");
}

/// The five encoding spaces are all present, with the counts the module
/// documentation claims.
///
/// This would catch a space truncated by an editing mistake — a `dd`
/// block that lost its `fd` twin, or a `bit` row that stopped at six bit
/// numbers. It would not catch a whole space being systematically wrong.
#[test]
fn all_five_encoding_spaces_are_present_in_full() {
    let space = |want: fn(&[u8]) -> bool| z80::TABLE.iter().filter(|insn| want(insn.code)).count();
    assert_eq!(space(|code| code.len() == 1), 252, "unprefixed");
    assert_eq!(space(|code| code.len() == 2 && code[0] == 0xCB), 248, "cb");
    assert_eq!(space(|code| code.len() == 2 && code[0] == 0xED), 58, "ed");
    assert_eq!(
        space(|code| code.len() == 2 && (code[0] == 0xDD || code[0] == 0xFD)),
        78,
        "dd and fd"
    );
    assert_eq!(space(|code| code.len() == 3), 62, "dd cb and fd cb");
    // The unprefixed space is every byte but the four prefixes.
    let mut opcodes: Vec<u8> = z80::TABLE
        .iter()
        .filter(|insn| insn.code.len() == 1)
        .map(|insn| insn.code[0])
        .collect();
    opcodes.sort_unstable();
    let expected: Vec<u8> = (0..=u8::MAX)
        .filter(|code| ![0xCB, 0xDD, 0xED, 0xFD].contains(code))
        .collect();
    assert_eq!(opcodes, expected);

    // The opcodes under one prefix, sorted, which is how the holes show.
    let under = |prefix: &[u8]| {
        let mut out: Vec<u8> = z80::TABLE
            .iter()
            .filter(|insn| insn.code.len() == prefix.len() + 1 && insn.code.starts_with(prefix))
            .map(|insn| insn.code[prefix.len()])
            .collect();
        out.sort_unstable();
        out
    };
    // The only holes in the `cb` space are the eight `sll` encodings.
    let cb: Vec<u8> = (0..=u8::MAX)
        .filter(|code| !(0x30..=0x37).contains(code))
        .collect();
    assert_eq!(under(&[0xCB]), cb, "the `cb` space");
    // IX and IY are the same 39 encodings twice over, and the `dd cb`
    // and `fd cb` spaces the same 31.
    assert_eq!(under(&[0xDD]), under(&[0xFD]), "`dd` against `fd`");
    assert_eq!(
        under(&[0xDD, 0xCB]),
        under(&[0xFD, 0xCB]),
        "`dd cb` against `fd cb`"
    );
    // Every `dd cb` opcode selects `(hl)` in the `cb` space, which is
    // what makes it an `(ix+d)` operation.
    for code in under(&[0xDD, 0xCB]) {
        assert_eq!(
            code & 0x07,
            0x06,
            "`dd cb {code:#04x}` is not an `(hl)` form"
        );
    }
}

/// Every entry is reachable through `find`, which is how a test that
/// builds a program by hand gets at it.
///
/// Two entries with the same mnemonic and the same shape would make the
/// second unreachable, which is how a table grows an encoding nothing can
/// ever emit; this fails on any such pair but the two documented
/// shadowed ones. It would not notice an entry that is reachable but
/// spelled in a shape no sane assembly programmer would write.
#[test]
fn every_entry_is_reachable_through_find() {
    let mut shadowed = 0;
    for insn in z80::TABLE {
        let found = z80::find(insn.name, insn.shape)
            .unwrap_or_else(|| panic!("`{}` is not in its own table", insn.syntax()));
        if found.code == insn.code {
            continue;
        }
        assert!(
            is_shadowed(insn),
            "`{}` is {:02x?} but `find` gives {:02x?}, and it is not one of the documented \
             shadowed encodings",
            insn.syntax(),
            insn.code,
            found.code
        );
        shadowed += 1;
    }
    assert_eq!(shadowed, SHADOWED.len(), "the shadowed encodings changed");
    for (name, shape, code) in SHADOWED {
        let short = z80::find(name, shape).expect("the short form");
        assert_eq!(short.code.len(), 1, "`{name} {shape}` keeps its short form");
        assert!(
            z80::TABLE
                .iter()
                .any(|insn| insn.name == *name && insn.shape == *shape && insn.code == *code),
            "the `ed` form of `{name} {shape}` is still in the table"
        );
    }
}

/// Every operand shape in the table assembles, from source text written
/// in that shape, to exactly the bytes the entry prints.
///
/// This is the main test: it walks all 698 entries, writes each one out
/// as assembly, assembles it and compares. It would catch a shape the
/// parser cannot parse, a literal field put in the wrong place (the
/// `dd cb` displacement especially), a length the assembler and the table
/// disagree about, and an operand spelling in the table that no source
/// text can produce. It would not catch an opcode that is wrong in both
/// the table and the reference reading it came from.
#[test]
fn every_shape_assembles_to_the_bytes_its_entry_prints() {
    let mut checked = 0;
    for insn in z80::TABLE {
        if is_shadowed(insn) {
            continue;
        }
        let source = format!("{}\nafter:\n", source_for(insn));
        let image = z80::assemble(&source)
            .unwrap_or_else(|why| panic!("`{}` did not assemble: {why}", insn.syntax()));
        let got = image.run(0x0000);
        assert_eq!(
            got,
            expected_for(insn),
            "`{}` assembled from `{}` to {got:02x?}",
            insn.syntax(),
            source.lines().next().unwrap_or_default()
        );
        checked += 1;
    }
    assert_eq!(checked, z80::TABLE.len() - SHADOWED.len());
}

/// The T-state counts the reference prints as a pair are recorded as a
/// pair, and no other instruction has one.
///
/// This pins the published numbers for the five families that have two
/// counts, so a later edit cannot quietly turn `jr nz` into a fixed
/// twelve. It would not catch a wrong single count: for that there is
/// only the reference and a part.
#[test]
fn the_instructions_with_two_counts_have_the_published_pair() {
    let pair = |name: &str, shape: &str| {
        let insn = z80::find(name, shape).unwrap_or_else(|| panic!("`{name} {shape}`"));
        (insn.tstates, insn.tstates_short)
    };
    for condition in ["nz", "z", "nc", "c"] {
        assert_eq!(pair("jr", &format!("{condition},label")), (12, Some(7)));
    }
    assert_eq!(pair("jr", "label"), (12, None));
    assert_eq!(pair("djnz", "label"), (13, Some(8)));
    for condition in ["nz", "z", "nc", "c", "po", "pe", "p", "m"] {
        assert_eq!(pair("call", &format!("{condition},nn")), (17, Some(10)));
        assert_eq!(pair("ret", condition), (11, Some(5)));
        // An unconditional jump costs the same either way, so the
        // reference prints one number for it.
        assert_eq!(pair("jp", &format!("{condition},nn")), (10, None));
    }
    assert_eq!(pair("call", "nn"), (17, None));
    assert_eq!(pair("ret", ""), (10, None));
    for block in [
        "ldir", "lddr", "cpir", "cpdr", "inir", "indr", "otir", "otdr",
    ] {
        assert_eq!(pair(block, ""), (21, Some(16)), "{block} repeating/final");
    }
    for block in ["ldi", "ldd", "cpi", "cpd", "ini", "ind", "outi", "outd"] {
        assert_eq!(pair(block, ""), (16, None), "{block}");
    }
    // Nothing else carries two counts, and where there are two the
    // short one is the shorter.
    let conditional: Vec<String> = z80::TABLE
        .iter()
        .filter(|insn| insn.is_conditional())
        .map(z80::Insn::syntax)
        .collect();
    assert_eq!(conditional.len(), 1 + 4 + 8 + 8 + 8, "{conditional:?}");
    for insn in z80::TABLE.iter().filter(|insn| insn.is_conditional()) {
        assert!(
            insn.tstates_short.expect("a pair") < insn.tstates,
            "`{}` has the pair the wrong way round",
            insn.syntax()
        );
    }
    // Every count is one the part can actually take: four T states for
    // the shortest instruction, twenty-three for the longest.
    for insn in z80::TABLE {
        assert!(
            (4..=23).contains(&insn.tstates),
            "`{}` takes {} T states",
            insn.syntax(),
            insn.tstates
        );
    }
}

/// A program that uses labels, every kind of literal field and all five
/// encoding spaces assembles byte for byte to what it should.
///
/// The bytes on the right were written out by hand from the same
/// reference the table came from, not copied from a run, so this is a
/// second reading of twenty-odd encodings rather than a recording of the
/// assembler's output. It would catch a backwards relative displacement
/// computed from the wrong address, a sixteen-bit literal emitted big
/// endian, a `(ix+d)` displacement in the wrong byte, and a `.db` or
/// `.dw` that places the wrong number of bytes. It would not catch
/// anything about the 670 encodings it does not mention.
#[test]
fn a_program_in_all_five_spaces_assembles_byte_for_byte() {
    let source = "\
        .org $0100\n\
                .equ count, 5\n\
                .equ vector, $38\n\
        start:  ld hl,table        ; unprefixed, sixteen-bit literal\n\
                ld de,$8000\n\
                ld bc,count\n\
                ld a,(hl)\n\
                ld ix,table\n\
                ld (ix+5),b        ; dd, with a displacement\n\
                ld b,(iy-2)        ; fd, negative displacement\n\
                bit 3,(iy-2)       ; fd cb\n\
                set 7,(ix+5)       ; dd cb\n\
                rlc (hl)           ; cb\n\
                sbc hl,bc          ; ed\n\
                ldir               ; ed, a block instruction\n\
                add hl,de\n\
                adc a,b\n\
                in a,($fe)\n\
                out ($fe),a\n\
                im 2\n\
                ex af,af'\n\
                jp (hl)\n\
        loop:   djnz loop          ; a backwards branch to itself\n\
                jr nz,loop\n\
                jr done\n\
                call vector\n\
                rst $38\n\
        done:   halt\n\
        table:  .db 1, 2, \"hi\", 0\n\
                .dw start, $1234\n";
    let image = match z80::assemble(source) {
        Ok(image) => image,
        Err(why) => panic!("{why}"),
    };
    assert_eq!(image.label("start"), 0x0100);
    assert_eq!(image.label("count"), 5);
    assert_eq!(image.label("vector"), 0x0038);
    assert!(image.has("table"));
    assert!(!image.has("missing"));

    #[rustfmt::skip]
    let expected: Vec<u8> = vec![
        0x21, 0x37, 0x01,       // ld hl,table   -- table is at $0137
        0x11, 0x00, 0x80,       // ld de,$8000
        0x01, 0x05, 0x00,       // ld bc,5
        0x7E,                   // ld a,(hl)
        0xDD, 0x21, 0x37, 0x01, // ld ix,table
        0xDD, 0x70, 0x05,       // ld (ix+5),b
        0xFD, 0x46, 0xFE,       // ld b,(iy-2)
        0xFD, 0xCB, 0xFE, 0x5E, // bit 3,(iy-2)
        0xDD, 0xCB, 0x05, 0xFE, // set 7,(ix+5)
        0xCB, 0x06,             // rlc (hl)
        0xED, 0x42,             // sbc hl,bc
        0xED, 0xB0,             // ldir
        0x19,                   // add hl,de
        0x88,                   // adc a,b
        0xDB, 0xFE,             // in a,($fe)
        0xD3, 0xFE,             // out ($fe),a
        0xED, 0x5E,             // im 2
        0x08,                   // ex af,af'
        0xE9,                   // jp (hl)
        0x10, 0xFE,             // djnz loop     -- back two, to itself
        0x20, 0xFC,             // jr nz,loop    -- back four
        0x18, 0x04,             // jr done      -- forward four
        0xCD, 0x38, 0x00,       // call $0038
        0xFF,                   // rst $38
        0x76,                   // halt
        0x01, 0x02, 0x68, 0x69, 0x00, // .db 1, 2, "hi", 0
        0x00, 0x01, 0x34, 0x12, // .dw start, $1234
    ];
    assert_eq!(image.run(0x0100), expected);
    assert_eq!(image.label("loop"), 0x012C);
    assert_eq!(image.label("done"), 0x0136);
    assert_eq!(image.label("table"), 0x0137);
    // Nothing was placed outside the program.
    assert_eq!(image.bytes.len(), expected.len());
    assert_eq!(image.bytes.keys().next(), Some(&0x0100));
}

/// `find`, `bytes` and `bytes_indexed` give the same bytes the assembler
/// does, for a test that would rather build a program than write one.
///
/// It would catch a `bytes` that forgot a prefix or put a displacement
/// after the opcode of a `dd cb` form. It would not catch a wrong opcode,
/// since both sides read the same table entry.
#[test]
fn the_by_hand_encoders_agree_with_the_assembler() {
    assert_eq!(z80::bytes("nop", "", 0), vec![0x00]);
    assert_eq!(z80::bytes("ld", "a,n", 0x12), vec![0x3E, 0x12]);
    assert_eq!(z80::bytes("ld", "hl,nn", 0x1234), vec![0x21, 0x34, 0x12]);
    assert_eq!(z80::bytes("in", "a,(n)", 0xFE), vec![0xDB, 0xFE]);
    assert_eq!(z80::bytes("ldir", "", 0), vec![0xED, 0xB0]);
    assert_eq!(z80::bytes("jr", "nz,label", 0xFE), vec![0x20, 0xFE]);
    assert_eq!(
        z80::bytes_indexed("ld", "(ix+d),b", 5, 0),
        vec![0xDD, 0x70, 0x05]
    );
    assert_eq!(
        z80::bytes_indexed("ld", "(ix+d),n", -2, 0x12),
        vec![0xDD, 0x36, 0xFE, 0x12]
    );
    assert_eq!(
        z80::bytes_indexed("bit", "3,(iy+d)", -2, 0),
        vec![0xFD, 0xCB, 0xFE, 0x5E]
    );
    assert!(z80::find("ld", "a,(ix+d)").is_some());
    assert!(z80::find("ld", "ixh,n").is_none(), "undocumented");
    assert!(z80::find("sll", "b").is_none(), "undocumented");
    assert!(z80::find("add", "b").is_none(), "rule 1: `add a,b`");
    assert!(z80::find("cp", "a,b").is_none(), "rule 1: `cp b`");
}

/// A number written any of the ways the assembler accepts means the same
/// thing, and a constant operand is matched by value.
///
/// This pins rule 5: `rst 56` and `rst $38` are one instruction. It would
/// catch a constant operand matched by its text instead of its value,
/// which would make `im 2` work and `im $02` not. It would not catch a
/// mistyped `rst` vector, since both spellings go through the same table
/// entry.
#[test]
fn a_constant_operand_is_matched_by_its_value() {
    for written in ["rst $38", "rst 56", "rst %111000", "rst 0x38"] {
        let image = z80::assemble(written).unwrap_or_else(|why| panic!("`{written}`: {why}"));
        assert_eq!(image.run(0), vec![0xFF], "{written}");
    }
    for written in ["im 2", "im $02", "bit 3,b", "bit $03,b"] {
        assert!(z80::assemble(written).is_ok(), "{written}");
    }
    assert_eq!(z80::assemble("bit 3,b").unwrap().run(0), vec![0xCB, 0x58]);
    assert_eq!(z80::assemble("bit $03,b").unwrap().run(0), vec![0xCB, 0x58]);
    // A defined name works where a literal does; an undefined one is an
    // error rather than a guess, because the first pass needs the value
    // to know which encoding this is.
    let defined = ".equ which, 3\nbit which,b\n";
    assert_eq!(z80::assemble(defined).unwrap().run(0), vec![0xCB, 0x58]);
    let forward = "bit later,b\n.equ later, 3\n";
    assert!(z80::assemble(forward).is_err(), "a forward constant");
    // Rule 8: `bit 8,b` is not an instruction.
    assert!(z80::assemble("bit 8,b").is_err());
}

/// Case and spacing do not change the bytes, and labels keep their case.
///
/// It would catch a parser that folded a label's case along with a
/// register's, which would make `Loop` and `loop` the same label. It
/// would not catch anything about the table.
#[test]
fn case_and_spacing_are_ignored_except_in_labels() {
    let loose = z80::assemble("  LD   A , ( IX + 5 )  ; shouting\n").expect("the loose spelling");
    let tight = z80::assemble("ld a,(ix+5)").expect("the tight spelling");
    assert_eq!(loose.run(0), vec![0xDD, 0x7E, 0x05]);
    assert_eq!(loose.run(0), tight.run(0));
    let both = z80::assemble("Loop: nop\nloop: nop\n").expect("two labels differing in case");
    assert_eq!(both.label("Loop"), 0);
    assert_eq!(both.label("loop"), 1);
}

/// `<` and `>` take the low and high byte of a value.
///
/// It would catch the two being swapped. It would not catch anything else.
#[test]
fn the_low_and_high_byte_operators() {
    let image = z80::assemble(".org $1234\nhere: ld a,<here\nld b,>here\n").expect("halves");
    assert_eq!(image.run(0x1234), vec![0x3E, 0x34, 0x06, 0x12]);
}

/// Malformed source is an `Err` naming the line, never a panic.
///
/// Every case here is a mistake a test program could plausibly contain.
/// This would catch an assembler that indexed or unwrapped its way out of
/// a bad operand — which is what a `.unwrap()` in a parser does — and an
/// error that forgot to say where it was. It would not prove the absence
/// of a panic on input nobody thought of; it is a list, not a proof.
#[test]
fn malformed_source_is_an_error_and_not_a_panic() {
    let bad = [
        ("frobnicate", "an unknown mnemonic"),
        ("ld", "a mnemonic with no operands at all"),
        ("ld a", "one operand where two belong"),
        ("ld a,b,c", "three operands where two belong"),
        ("ld a,", "a trailing comma"),
        ("ld ,b", "a leading comma"),
        ("ld q,b", "a register that does not exist"),
        ("ld a,(hl", "an unclosed parenthesis"),
        ("ld a,hl)", "a stray closing parenthesis"),
        ("ld a,(ix)", "rule 2: no displacement"),
        ("ld a,(ix+200)", "a displacement that does not fit"),
        ("ld a,(ix-200)", "a negative displacement that does not fit"),
        ("ld a,$1234", "an eight-bit field given sixteen bits"),
        ("add b", "rule 1: the accumulator left out"),
        ("cp a,b", "rule 1: the accumulator written twice"),
        ("jr nowhere", "a branch to a name nothing defines"),
        (".org $0000\njr $4000", "a branch that does not reach"),
        ("sll b", "an undocumented instruction"),
        ("ld ixh,$12", "an undocumented register half"),
        ("in f,(c)", "an undocumented encoding"),
        (".frobnicate 1", "an unknown directive"),
        (".org", "a directive with no operand"),
        (".org nowhere", "an origin from a name nothing defines"),
        (".equ name", "an `.equ` with no value"),
        (".equ a, 1", "rule 3: a register name as a label"),
        ("c: nop", "rule 3: a condition code as a label"),
        ("twice: nop\ntwice: nop", "a label defined twice"),
        (".db $1234", "a byte given sixteen bits"),
        (".db \"unterminated", "an unterminated string"),
        (".db \"\\q\"", "an unknown escape"),
        (".dw loop", "a word from a name nothing defines"),
        (".ds forever", "a `.ds` of an unknown length"),
        (".org $0000\nnop\n.org $0000\nnop", "a byte written twice"),
    ];
    for (source, why) in bad {
        let outcome = z80::assemble(source);
        assert!(
            outcome.is_err(),
            "`{}` is {why} and should not have assembled",
            source.escape_debug()
        );
        let message = outcome.err().expect("an error");
        assert!(!message.is_empty(), "`{source}` gave an empty message");
    }
    // The two that have to say which line they are on do.
    let message = match z80::assemble("nop\nnop\nfrobnicate\n") {
        Ok(_) => panic!("`frobnicate` assembled"),
        Err(message) => message,
    };
    assert!(message.starts_with("line 3:"), "{message}");
}

/// A program with holes in it stays sparse, and `.ds` leaves one.
///
/// It would catch a `.ds` that placed zeroes, which would make a test
/// comparing a whole image disagree for reasons that have nothing to do
/// with the program. It would not catch an `.org` that moved by the wrong
/// amount, which the byte-for-byte test above covers.
#[test]
fn an_org_and_a_ds_leave_holes() {
    let image = z80::assemble("nop\n.ds 4\nhere: nop\n.org $0200\nnop\n").expect("holes");
    assert_eq!(image.label("here"), 5);
    assert_eq!(image.run(0), vec![0x00]);
    assert_eq!(image.run(5), vec![0x00]);
    assert_eq!(image.run(0x0200), vec![0x00]);
    assert_eq!(image.bytes.len(), 3);
    assert_eq!(image.bytes.keys().copied().collect::<Vec<_>>(), [0, 5, 512]);
}
