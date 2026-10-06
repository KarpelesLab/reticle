; monitor.s — a machine-language monitor in two pages of ROM.
;
; ===================================================================
; THIS IS NOT ANYBODY ELSE'S MONITOR
; ===================================================================
;
; Apple's 1976 Woz Monitor is Apple's and Ben Eater's port of it is his;
; neither was read to write this, no disassembly or hex dump of either
; was consulted, and not one byte of either is in this directory. What
; is below was written from a *description* of the interface — the
; prompt, the items, the output format — and then **checked against a
; running original**, which is a different thing from copying one and is
; the whole point of the distinction. README.md says which behaviours
; were confirmed that way and which were only read about.
;
; What it reproduces is an interface, and an interface is a published
; fact about a machine rather than anybody's expression of it. That is
; the same position `examples/apple2` takes about the Apple II's memory
; map and soft switches, for the same reason, and that example's own
; monitor ROM (`../apple2/sw/monitor.s`) is where the hexadecimal
; printing, the line editor and the digit parser below come from: this
; is that code, cut down to fit a quarter of the space and re-aimed at a
; different interface.
;
; **Plain NMOS 6502.** No `DEC A`, no `PLX`, no `BRA`, no `STZ`:
; `ip/cpu/mos6502` is an NMOS part and says so, and the published 65C02
; ports of this interface do use those instructions — which is the one
; hard reason a real image of one cannot run on this machine.
; `tests/mos6502_monitor.rs` walks this ROM from every entry point and
; fails if any byte of it is not a documented NMOS encoding.
;
; ===================================================================
; THE MACHINE
; ===================================================================
;
;   $0000-$3FFF  RAM. Page zero is this program's variables, page one is
;                the stack, $0200 is the line being typed. The window is
;                16 KiB and the array behind it is smaller, so it
;                repeats — rtl/monitor_machine.v says by how much.
;   $4000-$5FFF  the ACIA, four registers over and over. $5000 is the
;                spelling everything uses:
;                  $5000 DATA     read: the byte received, and it is
;                                 taken by the read.
;                                 write: a byte to send.
;                  $5001 STATUS   bit 3 receiver data register full,
;                                 bit 4 transmit data register empty.
;                                 write: a programmed reset.
;                  $5002 COMMAND  data terminal ready, the interrupt
;                                 enables, parity.
;                  $5003 CONTROL  stop bits, word length, receiver clock
;                                 source, and the four-bit baud rate.
;   $6000-$7FFF  where the board's timer would be. Nothing here.
;   $8000-$FFFF  ROM. Only $FE00-$FFFF is built; see README.md.
;
; Plain seven-bit ASCII, because an ACIA passes what it is given and
; there is no keyboard latch setting bit 7.
;
; ===================================================================
; THE COMMANDS
; ===================================================================
;
; A line is parsed left to right as a series of *items*. An item is a
; run of hexadecimal digits, or one of `.`, `:` and `R`. Anything below
; `.` in the character set — a space, a comma, an asterisk — separates
; items and is otherwise ignored; anything else ends the line with a
; fresh `\` prompt.
;
; The parser has a mode, reset to EXAMINE at the start of every line and
; held in MODE as the character that set it:
;
;   EXAMINE  ($00) a number is an address: print it and the byte there,
;            and remember it as the place the next `.` or `:` works from.
;   BLOCK    (`.`) a number is the end of a range: print every byte from
;            one past the last one printed up to and including it.
;            Returns to EXAMINE once the range is printed.
;   STORE    (`:`) a number is a byte: write its low eight bits where the
;            last store left off. Stays until the line ends.
;
; and `R` runs from the address last examined.
;
; So:
;
;   FF00                  one byte
;   FFFA.FFFF             a range
;   0300: A9 41 60        a deposit
;   .0307                 more of the last range
;   : 60                  more of the last deposit
;   0300R                 run it
;
; Two consequences of that structure are quirks rather than decisions,
; and both are in the original: `XXXX:` prints the byte that *was* at
; XXXX before the deposit, because the item ends while the mode is still
; EXAMINE and the `:` changes it only afterwards; and `XXXXR` prints the
; byte at XXXX before jumping there, for the same reason. Both were
; confirmed against a running original.
;
; A new address label starts a line whenever the address has its low
; three bits clear, so a range comes out eight bytes to a row.
;
; Backspace is `$08`, the ASCII BS a terminal sends for its backspace
; key. Escape ($1B) cancels the line. README.md has the argument about
; the original's `_`. A line stops growing at 128 characters: the 129th
; replaces the 128th rather than being refused, which is the behaviour
; that costs no bytes and loses no line.
;
; ===================================================================
; WHAT IS WHERE, AND WHY IT IS WRITTEN THIS WAY
; ===================================================================
;
; **This is 266 bytes of code and six of vectors, and the interface it
; reproduces fits 256 in all.** The target was one page, because the ROM
; on this part is lookup tables and the original fits one for a version
; of the same reason; it did not fit, it was measured, and README.md
; reports what the second page costs — 505 lookup tables for 512 bytes,
; one a byte, so about 250 of the part's 12 144 for the page that was not
; needed. Some of the difference is deliberate: `echo` polls the
; transmitter where the original stores a byte and spins a delay loop.
; The rest is the price of writing a parser that can be read.
;
; Six things below are shorter than they would otherwise be, and each is
; marked where it happens:
;
;   * **X is zero from the start of a line to the end of it.** That
;     makes `sta (STL,x)` and `lda (XAML,x)` reach a pointer in two
;     bytes each instead of four, and `stx` clear a variable in two
;     instead of four — and, more importantly, it means nothing in the
;     parser clobbers Y, which is the position in the line.
;   * The four shifts that move a digit in are a **loop**, not four
;     pairs of instructions.
;   * `show`, `prbyte`, `prnib` and `echo` are **one chain of
;     fall-throughs**: the last instruction of each is the first of the
;     next, so four routines cost three `jmp`s less than four routines.
;   * MODE holds **the character that set it**, so setting it is one
;     `sta` rather than a constant and a store.
;   * `setmode` sits immediately above `item` and **falls into it**,
;     which is two bytes the branch back would have cost.
;   * The digit test is the ordinary `eor #$30` idiom rather than four
;     range comparisons. Its working is written out where it is.
;
; rtl/monitor_rom.v is this file assembled. tests/mos6502_monitor.rs
; assembles it again and fails if the two differ; run it with
; UPDATE_EXPECT=1 to rewrite the ROM after editing this file.

; ---------------------------------------------------------------------
; Page zero. XAML/XAMH and STL/STH are a pair each and adjacent on
; purpose: `(XAML,x)` and `(STL,x)` are the only way a 6502 reaches a
; computed address, and page zero is the only page they can point from.
; ---------------------------------------------------------------------
        .equ XAML,  $00     ; the address last examined
        .equ XAMH,  $01
        .equ STL,   $02     ; the address the next deposit goes to
        .equ STH,   $03
        .equ VALL,  $04     ; the number being read out of the line
        .equ VALH,  $05
        .equ MODE,  $06     ; $00 examine, `.` block, `:` store
        .equ SAVY,  $07     ; where in the line the current item began

        .equ IN,    $0200   ; the line being typed, 128 bytes

        .equ ACIAD, $5000   ; data
        .equ ACIAS, $5001   ; status
        .equ ACIAC, $5002   ; command
        .equ ACIAX, $5003   ; control

        .org $fe00

; ---------------------------------------------------------------------
; Reset. The 6502 arrives here from $FFFC, seven cycles after the board
; lets go of reset, with nothing set up at all.
;
; `$1F` in CONTROL is the ACIA's documented initialisation and the value
; a published port of this interface writes, because there is only one
; sensible answer: one stop bit, eight data bits, the receiver clocked
; from the baud rate generator, and baud code 15.
;
; Baud code 15 is 19200 on a 65C51, and this machine's console is a USB
; pipe with no bit rate at all — so the *hardware* overwrites those four
; bits with whatever the host asked for the moment the host asks, which
; is a thing a USB serial bridge does and a 65C51 on a breadboard
; cannot. rtl/monitor_acia.v says exactly when, and README.md says why
; that is the honest arrangement rather than a fudge.
;
; COMMAND is left at its reset value and the status register is not
; written, which a real initialisation would do: neither has any effect
; on this ACIA — every bit of COMMAND is stored and not acted on — and
; the eight bytes they cost are eight bytes this page does not have.
; ---------------------------------------------------------------------
reset:
        ldx #$ff
        txs                     ; the stack at $01FF, growing down
        lda #$1f
        sta ACIAX

; ---------------------------------------------------------------------
; The prompt. A backslash, and then a fresh line.
; ---------------------------------------------------------------------
escape:
        lda #$5c                ; `\`
        jsr echo

; ---------------------------------------------------------------------
; Read a line into IN. `rdkey` echoes, so everything a person types
; comes back from the machine and not from their kernel.
; ---------------------------------------------------------------------
getline:
        lda #$0d                ; a carriage return before every line
        jsr echo
        ldy #$00
nextchar:
        jsr rdkey
        cmp #$1b                ; escape cancels the line
        beq escape
        cmp #$08                ; backspace
        beq backsp
        sta IN,y
        cmp #$0d                ; the return is stored, and ends the line
        beq parse
        iny
        bpl nextchar
        ; 128 characters and no return. Fall into the backspace, which
        ; steps back over the one just stored: the line stops growing
        ; instead of being thrown away.
backsp:
        dey
        bpl nextchar
        iny                     ; already at the left margin: undo it
        bpl nextchar            ; always, because Y is now zero

; ---------------------------------------------------------------------
; Parse the line. One item at a time; `act` does what MODE says with the
; number, and the character that ended the item then says what MODE
; becomes.
;
; X is set to zero here and stays zero for the whole line. Nothing below
; changes it except the shift loop, which ends at zero, so every
; `(zp,x)` below is `(zp)` and Y is never needed as an index into page
; zero — which is what leaves Y free to be the position in the line.
; ---------------------------------------------------------------------
parse:
        ldx #$00
        stx MODE                ; every line starts out examining
        txa
        tay
setmode:
        sta MODE                ; the character that set it *is* the mode,
                                ; and it falls straight into the next item
item:
        stx VALL
        stx VALH
        sty SAVY                ; where this item began
digit:
        lda IN,y
        ; A hexadecimal digit, or not.
        ;
        ;   eor #$30  maps `0`-`9` ($30-$39) to $00-$09, and `A`-`F`
        ;             ($41-$46) to $71-$76.
        ;   cmp #$0a  separates the first group, which is finished.
        ;   adc #$88  runs on the second with carry set, so it adds $89
        ;             and takes $71-$76 to $FA-$FF. Everything else
        ;             either lands below $FA or wraps past $FF and lands
        ;             far below it, and `A`-`F` are the only characters
        ;             that can reach $FA-$FF at all.
        ;   and #$0f  is then the value, for both groups.
        eor #$30
        cmp #$0a
        bcc gotdig
        adc #$88
        cmp #$fa
        bcc enditem
gotdig:
        and #$0f
        ldx #$04                ; four shifts, and X back to zero after
shift:
        asl VALL
        rol VALH
        dex
        bne shift
        ora VALL
        sta VALL
        iny
        bne digit               ; always: Y is under 128

enditem:
        cpy SAVY
        beq noval               ; a bare `.`, `:` or `R` has no number
        jsr act
noval:
        lda IN,y
        iny
        cmp #$0d
        beq getline             ; the line is finished
        cmp #$2e                ; `.`
        bcc item                ; below it: a separator, so the next item
        beq setmode
        cmp #$3a                ; `:`
        beq setmode
        cmp #$52                ; `R`
        beq run
        jmp escape              ; anything else: the line is not a line
run:
        jmp (XAML)              ; and never come back

; ---------------------------------------------------------------------
; What a number means, which is what MODE says.
; ---------------------------------------------------------------------
act:
        lda MODE
        beq doxam
        cmp #$2e
        beq doblk

        ; STORE: the low byte, where the last deposit left off.
        lda VALL
        sta (STL,x)
        inc STL
        bne act_rts
        inc STH
act_rts:
        rts

        ; EXAMINE: this is now both the place a range starts and the
        ; place a deposit goes, and it gets an address label of its own
        ; however its low bits fall — which is what `header` is, entered
        ; past the test `show` makes.
doxam:
        lda VALL
        sta XAML
        sta STL
        lda VALH
        sta XAMH
        sta STH
        jmp header

        ; BLOCK: up to and including the number, then back to examining.
        ; The comparison is sixteen bits, so a range that runs backwards
        ; prints nothing rather than sixty-four kilobytes.
doblk:
        lda XAML
        cmp VALL
        lda XAMH
        sbc VALH
        bcs blk_end
        inc XAML
        bne blk_1
        inc XAMH
blk_1:
        jsr show
        jmp doblk
blk_end:
        stx MODE                ; X is zero: back to examining
        rts

; ---------------------------------------------------------------------
; One byte of a dump, and then everything that prints anything. The four
; routines fall into each other in the order a byte travels: `show`
; decides whether a row starts here, `header` writes the address,
; `prbyte` writes two digits, `prnib` writes one, `echo` writes a
; character.
; ---------------------------------------------------------------------
show:
        lda XAML
        and #$07
        bne body                ; not the start of a row
header:
        lda #$0d
        jsr echo
        lda XAMH
        jsr prbyte
        lda XAML
        jsr prbyte
        lda #$3a                ; `:`
        jsr echo
body:
        lda #$20                ; a space before every byte
        jsr echo
        lda (XAML,x)
        ; fall into prbyte

prbyte:
        pha
        lsr a
        lsr a
        lsr a
        lsr a
        jsr prnib
        pla
        and #$0f
        ; fall into prnib
prnib:
        ora #$30                ; `0`
        cmp #$3a                ; past `9`?
        bcc echo
        adc #$06                ; carry is set here, so this adds seven
        ; fall into echo

; echo(A): one character out, once the transmitter says it will take
; one. A is preserved, because every caller above relies on it.
;
; The published port of this interface does **not** poll: it stores the
; byte and then spins a delay loop long enough for the byte to have
; gone. That works and it is not worth copying — a design that is
; correct only because the software wastes time has nowhere to go when
; the rate changes, and the rate here changes whenever a host says so.
echo:
        pha
echo_1:
        lda ACIAS
        and #$10                ; transmit data register empty
        beq echo_1
        pla
        sta ACIAD
        rts

; rdkey: wait for a byte, take it, and echo it. Bit 3 of STATUS is "the
; receiver has one" and reading DATA takes it, which is the whole
; protocol. The echo is here rather than at the one place that calls
; this, because `echo` hands A back untouched and a tail call is one
; byte less than a call and a return.
rdkey:
        lda ACIAS
        and #$08                ; receiver data register full
        beq rdkey
        lda ACIAD
        jmp echo

; ---------------------------------------------------------------------
; Nothing in this machine raises an interrupt and there is no room for a
; handler, so NMI and IRQ point at the reset entry: an interrupt that
; cannot happen restarts the machine rather than running whatever $00
; is. BRK does the same, which is a usable answer for a program run with
; `R` that falls off the end of itself.
; ---------------------------------------------------------------------
nmi:
irq:

; The three vectors, at the top of the ROM and therefore at the top of
; the address space. $FFFC is read before any RAM can have been written,
; which is why the ROM has to be here. Everything between the end of the
; program and $FFFA is a hole the ROM answers with zero.
        .org $fffa
        .word reset             ; $FFFA  NMI
        .word reset             ; $FFFC  RES
        .word reset             ; $FFFE  IRQ and BRK
