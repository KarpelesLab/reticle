; monitor.s — the machine's ROM: a screen, a keyboard and three commands.
;
; This is not Apple's Monitor, or a disassembly of it, or a rewrite of it.
; Apple's Monitor ROM, Integer BASIC, Applesoft and the character
; generator are all still Apple's; none of them is here, none of them was
; read to write this, and this machine does not need any of them. What is
; below was written for this example: a cursor, a screen driver, a line
; editor and a command loop, in just over a kilobyte.
;
; It knows the machine that rtl/apple2.v decodes:
;
;   $0000-$000F  this program's variables
;   $0200-$0227  the line being typed
;   $0400-$07FF  the text page, whose lines are *not* in order — see
;                `linelo` at the bottom of this file
;   $C000        KBD      bit 7 is "a key is waiting", bits 6..0 the key
;   $C010        KBDSTRB  touching it says the key has been taken
;   $C030        SPKR     touching it moves the speaker one way
;   $C0A0        the serial card: write a byte to send it
;   $C0A1        its status, bit 0 = it will take a byte now
;   $F800-$FFFF  this
;
; Everything printed goes to the screen *and* to the serial port, so the
; terminal that is the keyboard is also a transcript. That is what lets
; tests/apple2.rs read the session off the serial wire as well as off the
; video signal, and the two have to agree.
;
; The commands, typed at the `]` prompt:
;
;   E <addr>              print eight bytes from <addr>
;   D <addr> <b> <b> ...  write bytes to <addr>
;   T                     paint the character test card over the rest of
;                         the screen
;
; Anything else gets a `?` and a beep. There is no graphics command
; because there are no graphics: see README.md.
;
; monitor.hex is this file assembled. tests/apple2.rs assembles it again
; and fails if the two differ; run it with UPDATE_EXPECT=1 to rewrite the
; hex after editing this file.

; ---------------------------------------------------------------------
; Variables. Page zero, because on a 6502 that is where variables go: it
; is the only page with a one-byte address, and `(ptr),y` can only point
; through it.
; ---------------------------------------------------------------------
        .equ CH,      $00     ; cursor column, 0..39
        .equ CV,      $01     ; cursor row, 0..23
        .equ BASL,    $02     ; the address of column 0 of row CV
        .equ BASH,    $03
        .equ SRCL,    $04     ; the row being copied from, while scrolling
        .equ SRCH,    $05
        .equ PTRL,    $06     ; the address a command is working on
        .equ PTRH,    $07
        .equ VALL,    $08     ; the number `gethex` last read
        .equ VALH,    $09
        .equ IDX,     $0a     ; where the parser is in the input line
        .equ LEN,     $0b     ; how long the input line is
        .equ CNT,     $0c
        .equ TMP,     $0d
        .equ ROW,     $0e
        .equ CHAR,    $0f     ; the character `cout` is printing
        .equ STRL,    $10     ; the string `putstr` is writing
        .equ STRH,    $11
        .equ MASK,    $12     ; and how it turns ASCII into a display code:
        .equ SETB,    $13     ; AND with MASK, then OR with SETB

        .equ INBUF,   $0200   ; the line being typed, 40 bytes

; ---------------------------------------------------------------------
; The soft switches. Reading one *is* the operation: there is nothing
; stored at these addresses, so `bit` is the cheapest way to touch one
; without disturbing A.
; ---------------------------------------------------------------------
        .equ KBD,     $c000
        .equ KBDSTRB, $c010
        .equ SPKR,    $c030
        .equ SERDATA, $c0a0
        .equ SERSTAT, $c0a1

        .equ SPACE,   $a0     ; the display code of a normal space
        .equ CURSOR,  $5f     ; a flashing `_`: $40 makes it flash, $1f
                              ; is the glyph of ASCII $5f

        .org $f800

; ---------------------------------------------------------------------
; Reset. The 6502 arrives here from $FFFC, seven cycles after the board
; lets go of reset, with nothing set up at all.
; ---------------------------------------------------------------------
reset:
        sei                     ; nothing raises an interrupt; say so
        cld                     ; RES does not define D on a real part
        ldx #$ff
        txs                     ; the stack at $01FF, growing down
        bit KBDSTRB             ; whatever the keyboard thinks, forget it
        jsr clrscr
        lda #$00
        sta CH
        sta CV
        jsr setbas
        jsr banner
        jsr beep                ; a machine that has just woken up says so

; ---------------------------------------------------------------------
; The command loop.
; ---------------------------------------------------------------------
mainlp:
        lda #$5d                ; `]`
        jsr cout
        jsr getln
        jsr docmd
        jmp mainlp

; ---------------------------------------------------------------------
; The screen
; ---------------------------------------------------------------------

; clrscr: every byte of the text page becomes a normal space, including
; the sixty-four the line table never reaches. Four stores per pass
; because the page is four pages.
clrscr:
        lda #SPACE
        ldx #$00
clr1:
        sta $0400,x
        sta $0500,x
        sta $0600,x
        sta $0700,x
        inx
        bne clr1
        rts

; setbas: BASL/BASH = the address of column 0 of row CV.
setbas:
        ldx CV
        lda linelo,x
        sta BASL
        lda linehi,x
        sta BASH
        rts

; setpos: BASL/BASH = row X, column A. The column can be added to the
; low byte alone: a row's base is $x00, $x28, $x50, $x80, $xa8 or $xd0 and
; a column is less than 40, so the sum never leaves the page.
setpos:
        clc
        adc linelo,x
        sta BASL
        lda linehi,x
        sta BASH
        rts

; putstr: write the NUL-terminated string at STRL to the screen from
; BASL, turning each byte into a display code with `and MASK` then
; `ora SETB`, and sending the plain ASCII to the terminal. BASL is left
; past the end of what was written, so the next call carries on.
;
; The three attributes are three pairs of constants and nothing else:
;   normal    MASK $ff  SETB $80
;   inverse   MASK $3f  SETB $00
;   flashing  MASK $3f  SETB $40
putstr:
        ldy #$00
ps1:
        lda (STRL),y
        beq ps2
        pha
        and MASK
        ora SETB
        sta (BASL),y
        pla
        jsr serout
        iny
        bne ps1
ps2:
        tya
        clc
        adc BASL
        sta BASL
        bcc ps3
        inc BASH
ps3:
        rts

; beep: the speaker, which on an Apple II is one bit and nothing else.
; Touching $C030 moves the cone one way; touching it again moves it back,
; so a note is a loop and its pitch is how long the loop takes. Four
; periods of about 1010 cycles each: 890 Hz for four and a half
; milliseconds at the 1.798 MHz the machine runs at, which is a blip
; rather than a note, because every cycle of it is also a cycle of
; simulation. It is what tests/apple2.rs listens for.
beep:
        ldx #$08                ; four periods
bp1:
        bit SPKR
        ldy #$c8
bp2:
        dey
        bne bp2
        dex
        bne bp1
        rts

; serln: end a line on the terminal only.
serln:
        lda #$0d
        jsr serout
        lda #$0a
        jmp serout

; curon / curoff: draw or erase the cursor where it is now.
curon:
        lda #CURSOR
        ldy CH
        sta (BASL),y
        rts

curoff:
        lda #SPACE
        ldy CH
        sta (BASL),y
        rts

; newline: column 0 of the next row, scrolling if there is no next row.
newline:
        lda #$00
        sta CH
        inc CV
        lda CV
        cmp #24
        bcc nl1
        jsr scroll
        lda #23
        sta CV
nl1:
        jmp setbas

; scroll: rows 1..23 move up one and row 23 is cleared. Forty stores a
; row through two zero-page pointers, which is what page zero is for.
scroll:
        lda #$00
        sta ROW
scr_row:
        ldx ROW
        lda linelo,x
        sta BASL
        lda linehi,x
        sta BASH
        inx
        lda linelo,x
        sta SRCL
        lda linehi,x
        sta SRCH
        ldy #39
scr_col:
        lda (SRCL),y
        sta (BASL),y
        dey
        bpl scr_col
        inc ROW
        lda ROW
        cmp #23
        bcc scr_row
        ldx #23
        lda linelo,x
        sta BASL
        lda linehi,x
        sta BASH
        lda #SPACE
        ldy #39
scr_clr:
        sta (BASL),y
        dey
        bpl scr_clr
        rts

; cout(A = ASCII): to the screen at the cursor, and to the terminal.
; Carriage return moves the cursor down; everything else is printed as
; the normal display code of itself, which for ASCII $20..$5F is the
; ASCII with bit 7 set.
cout:
        and #$7f
        sta CHAR
        cmp #$0d
        beq coutcr
        ora #$80
        ldy CH
        sta (BASL),y
        inc CH
        lda CH
        cmp #40
        bcs coutwrap
        jsr curon
        jmp coutser
coutwrap:
        jsr newline
        jsr curon
coutser:
        lda CHAR
        jmp serout
coutcr:
        jsr curoff
        jsr newline
        jsr curon
        lda #$0d
        jsr serout
        lda #$0a                ; a terminal wants the line feed too
        jmp serout

; crout: one carriage return.
crout:
        lda #$0d
        jmp cout

; serout(A): hand a byte to the serial card once it will take one.
serout:
        pha
sero1:
        lda SERSTAT
        and #$01
        beq sero1
        pla
        sta SERDATA
        rts

; prnib(A = 0..15) and prbyte(A): hexadecimal, through cout.
prnib:
        cmp #$0a
        bcc prn1
        adc #$06                ; carry is set here, so this adds seven
prn1:
        adc #$30
        jmp cout

prbyte:
        pha
        lsr a
        lsr a
        lsr a
        lsr a
        jsr prnib
        pla
        and #$0f
        jmp prnib

; prword: PTRH:PTRL as four hexadecimal digits.
prword:
        lda PTRH
        jsr prbyte
        lda PTRL
        jmp prbyte

; ---------------------------------------------------------------------
; The keyboard
; ---------------------------------------------------------------------

; rdkey: wait for a key, take it, fold lower case up. The strobe is bit 7
; of $C000 and touching $C010 clears it, which is the whole protocol.
rdkey:
        lda KBD
        bpl rdkey
        bit KBDSTRB
        and #$7f
        cmp #$61                ; `a`
        bcc rdk1
        cmp #$7b                ; past `z`
        bcs rdk1
        and #$5f                ; clear bit 5: `a` becomes `A`
rdk1:
        rts

; getln: read a line into INBUF, echoing it, until a carriage return.
; Backspace erases; a full line ignores anything but those two.
getln:
        ldx #$00
getl1:
        jsr rdkey
        cmp #$0d
        beq getl2
        cmp #$08
        beq getl_bs
        cpx #39
        bcs getl1
        sta INBUF,x
        inx
        jsr cout
        jmp getl1
getl_bs:
        cpx #$00
        beq getl1
        dex
        jsr bsout
        jmp getl1
getl2:
        stx LEN
        jmp cout                ; echo the return, which moves the cursor

; bsout: take the cursor back one column, over the character it deletes,
; and tell the terminal to do the same.
bsout:
        lda CH
        beq bs_done
        jsr curoff
        dec CH
        jsr curon
        lda #$08
        jsr serout
        lda #$20
        jsr serout
        lda #$08
        jsr serout
bs_done:
        rts

; ---------------------------------------------------------------------
; The commands
; ---------------------------------------------------------------------
; A command is dispatched with `jmp` rather than `beq`, because a branch
; reaches 127 bytes and the four commands are longer than that put
; together.
docmd:
        lda LEN
        beq dc_done
        lda INBUF
        cmp #$45                ; E
        bne dc_d
        jmp do_exam
dc_d:
        cmp #$44                ; D
        bne dc_t
        jmp do_dep
dc_t:
        cmp #$54                ; T
        bne dc_err
        jmp do_test
dc_err:
        jsr beep
        lda #$3f                ; `?`
        jsr cout
        jmp crout
dc_done:
        rts

; E <addr>: eight bytes, as `AAAA: bb bb bb bb bb bb bb bb`.
do_exam:
        jsr argaddr
        bcc dc_err
        jsr prword
        lda #$3a                ; `:`
        jsr cout
        lda #$00
        sta CNT
dmp1:
        lda #$20
        jsr cout
        ldy CNT
        lda (PTRL),y
        jsr prbyte
        inc CNT
        lda CNT
        cmp #$08
        bcc dmp1
        jmp crout

; D <addr> <byte> ...: as many bytes as are written.
do_dep:
        jsr argaddr
        bcc dc_err
        lda #$00
        sta CNT
dep1:
        jsr skipsp
        jsr gethex
        bcc dep_end
        ldy CNT
        lda VALL
        sta (PTRL),y
        inc CNT
        jmp dep1
dep_end:
        rts

; argaddr: the address after the command letter, into PTRL/PTRH. Carry
; clear means there was not one.
argaddr:
        lda #$01
        sta IDX
        jsr skipsp
        jsr gethex
        bcc arg_no
        lda VALL
        sta PTRL
        lda VALH
        sta PTRH
        sec
        rts
arg_no:
        clc
        rts

; T: fill the rest of the screen with every glyph the character
; generator has, shifted by seven each row so that no two rows are alike.
; That is deliberate: it is what lets a test tell the interleaved line
; order apart from any other order.
do_test:
        lda CV
        sta ROW
tc_row:
        lda ROW
        cmp #24
        bcs tc_done
        ldx ROW
        lda linelo,x
        sta BASL
        lda linehi,x
        sta BASH
        lda #$00
        ldx #$07                ; the first glyph of the row is 7 * row
tc_mul:
        clc
        adc ROW
        dex
        bne tc_mul
        sta TMP
        ldy #$00
tc_col:
        lda TMP
        and #$3f
        ora #$80                ; normal, not inverse and not flashing
        sta (BASL),y
        inc TMP
        iny
        cpy #40
        bcc tc_col
        inc ROW
        jmp tc_row
tc_done:
        jmp setbas              ; BASL was borrowed; put it back

; ---------------------------------------------------------------------
; Reading the line
; ---------------------------------------------------------------------

; skipsp: move IDX past any spaces.
skipsp:
        ldy IDX
sk1:
        cpy LEN
        bcs sk2
        lda INBUF,y
        cmp #$20
        bne sk2
        iny
        jmp sk1
sk2:
        sty IDX
        rts

; gethex: read hexadecimal digits at IDX into VALH:VALL. Carry set if
; there was at least one digit.
gethex:
        lda #$00
        sta VALL
        sta VALH
        ldx #$00
gh1:
        ldy IDX
        cpy LEN
        bcs gh_end
        lda INBUF,y
        jsr hexval
        bcc gh_end
        asl VALL
        rol VALH
        asl VALL
        rol VALH
        asl VALL
        rol VALH
        asl VALL
        rol VALH
        ora VALL
        sta VALL
        inc IDX
        inx
        jmp gh1
gh_end:
        cpx #$00
        beq gh_no
        sec
        rts
gh_no:
        clc
        rts

; hexval(A = a character): its value with carry set, or carry clear.
hexval:
        cmp #$30                ; `0`
        bcc hv_no
        cmp #$3a
        bcs hv_alpha
        sec
        sbc #$30
        sec
        rts
hv_alpha:
        cmp #$41                ; `A`
        bcc hv_no
        cmp #$47                ; past `F`
        bcs hv_no
        sec
        sbc #$37
        sec
        rts
hv_no:
        clc
        rts

; ---------------------------------------------------------------------
; The opening screen
; ---------------------------------------------------------------------

; banner: three rows.
;
;   0  the title, inverse, right across the screen
;   1  one word in each of the three attributes, so that a look at the
;      screen — or a test that decodes it — sees all three at once
;   2  what the machine is, through `cout`, so the cursor ends up under it
banner:
        ldx #$00
        lda #$00
        jsr setpos
        lda #$3f                ; inverse
        sta MASK
        lda #$00
        sta SETB
        lda #<title
        sta STRL
        lda #>title
        sta STRH
        jsr putstr
        jsr serln

        ldx #$01
        lda #$00
        jsr setpos
        lda #$ff                ; normal
        sta MASK
        lda #$80
        sta SETB
        lda #<attnorm
        sta STRL
        lda #>attnorm
        sta STRH
        jsr putstr
        lda #$3f                ; inverse
        sta MASK
        lda #$00
        sta SETB
        lda #<attinv
        sta STRL
        lda #>attinv
        sta STRH
        jsr putstr
        lda #$3f                ; flashing
        sta MASK
        lda #$40
        sta SETB
        lda #<attflash
        sta STRL
        lda #>attflash
        sta STRH
        jsr putstr
        jsr serln

        lda #$00
        sta CH
        sta IDX
        lda #$02
        sta CV
        jsr setbas
ban3:
        ldy IDX
        lda info,y
        beq ban4
        jsr cout
        inc IDX
        jmp ban3
ban4:
        jmp crout

title:
        .string "    RETICLE APPLE ][ COMPATIBLE 6502    "
attnorm:
        .string "ATTRIBUTES: NORMAL "
attinv:
        .string "INVERSE "
attflash:
        .string "FLASHING"
info:
        .string "TEXT 40X24  $0400 INTERLEAVED"

; ---------------------------------------------------------------------
; The line table: the address of column 0 of each row.
;
; Row N is at $0400 + 128 * (N mod 8) + 40 * (N div 8), which is the
; address an Apple II's video counter produces and the reason the screen
; is three interleaved groups of eight lines rather than one block of
; twenty-four. tests/apple2.rs computes these forty-eight bytes from that
; formula and compares them with what is assembled here, so the table
; cannot quietly drift into being some other order.
; ---------------------------------------------------------------------
linelo:
        .byte $00,$80,$00,$80,$00,$80,$00,$80
        .byte $28,$a8,$28,$a8,$28,$a8,$28,$a8
        .byte $50,$d0,$50,$d0,$50,$d0,$50,$d0
linehi:
        .byte $04,$04,$05,$05,$06,$06,$07,$07
        .byte $04,$04,$05,$05,$06,$06,$07,$07
        .byte $04,$04,$05,$05,$06,$06,$07,$07

; ---------------------------------------------------------------------
; Nothing in this machine raises an interrupt and I is set above, but the
; vectors have to point at instructions rather than at whatever $00 is.
; ---------------------------------------------------------------------
nmi:
irq:
        rti

; The three vectors, at the top of the ROM and therefore at the top of
; the address space. $FFFC is read before any RAM can have been written,
; which is why the ROM has to be here.
        .org $fffa
        .word nmi               ; $FFFA  NMI
        .word reset             ; $FFFC  RES
        .word irq               ; $FFFE  IRQ and BRK
