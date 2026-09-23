; demo.s — the program on the demo cartridge.
;
; A scrolling landscape with a few sprites walking across it and a colour
; that changes as the frames go by. Everything it draws is in this file
; and in sw/chr.s: no part of any commercial game is here, and none is
; needed — this is a program written for this console, the way any other
; program for it would be written.
;
; What it does, in order:
;
;   1. sets the stack up and turns the picture unit off;
;   2. fills the page of work RAM that sprite DMA copies from;
;   3. writes the palette;
;   4. fills both nametables — one tile per row from `rows`, then the
;      attribute table — and then pokes a few individual tiles over the
;      result from `decor`, which is what puts the stars, the clouds and
;      the gems where they are;
;   5. copies the sprites into OAM with a DMA, sets the scroll to zero
;      and turns the picture on with the NMI enabled;
;   6. does nothing else. Every frame after that happens in `nmi`, which
;      moves the sprites one pixel right, DMAs them in, advances the
;      colour of the stars, and sets the scroll to the frame number — so
;      frame N is drawn with the background scrolled N pixels left and
;      the sprites N pixels right of where they started.
;
; **It does not wait for the picture unit to warm up.** A program for a
; real console reads $2002 until vblank twice before it touches anything,
; because a real 2C02 ignores writes for about 30,000 cycles after power
; on. This one is ready the moment reset is released, so the wait would
; buy nothing and would cost a whole frame of simulation in every test.
; That is the one place this program would need a change to run on real
; silicon, and it is written here so nobody has to find it.
;
; The memory map is the one rtl/nes_console.v decodes:
;
;   $0000-$07FF  work RAM, mirrored to $1FFF. $0200 is the sprite page.
;   $2000-$2007  the picture unit, mirrored to $3FFF
;   $4014        sprite DMA
;   $8000-$FFFF  this ROM, 16 KiB seen twice, vectors at the very top
;
; demo.hex is this file assembled. tests/nes.rs assembles it again and
; fails if the two differ; run it with UPDATE_EXPECT=1 to rewrite the hex
; after editing this file.

        .equ PPUCTRL,   $2000
        .equ PPUMASK,   $2001
        .equ PPUSTATUS, $2002
        .equ OAMADDR,   $2003
        .equ PPUSCROLL, $2005
        .equ PPUADDR,   $2006
        .equ PPUDATA,   $2007
        .equ OAMDMA,    $4014

; $2000: NMI on, sprites out of pattern table 1, background out of 0,
; base nametable 0.
        .equ CTRL_ON,   $88
; $2001: background and sprites shown, both of them in the leftmost
; eight pixels too.
        .equ MASK_ON,   $1e

; Zero page. Two bytes is the whole of this program's state.
        .equ frame,     $10

; The page sprite DMA copies from, four bytes a sprite.
        .equ OAM_PAGE,  $02
        .equ oam,       $0200

; NROM-128: 16 KiB of ROM answering at $8000 and again at $C000. The
; program is assembled in the upper copy so that the vectors, which the
; part reads from $FFFA-$FFFF, are simply its last six bytes.
        .org $C000

; ---------------------------------------------------------------------
; Reset
; ---------------------------------------------------------------------
reset:
        sei
        cld
        ldx #$ff
        txs

        ; Nothing on the screen while the memory behind it is written.
        lda #$00
        sta PPUCTRL
        sta PPUMASK
        sta frame

        ; The sprite page, every byte of it. Nothing has written this
        ; RAM, and a sprite whose Y is not a number is a sprite that
        ; could be anywhere: $F8 puts one on line 249, which is off the
        ; bottom of a 240-line picture, so the 60 sprites this demo does
        ; not use are invisible rather than undefined.
        lda #$f8
        ldx #$00
clear_oam:
        sta oam,x
        inx
        bne clear_oam

        ; ...and then the four it does use.
        ldx #$00
copy_sprites:
        lda sprites,x
        sta oam,x
        inx
        cpx #16
        bne copy_sprites

        ; The palette, at $3F00. `bit` reads $2002, which puts the write
        ; latch back to its first half — the one rule of $2006 and $2005
        ; that a program cannot skip.
        bit PPUSTATUS
        lda #$3f
        sta PPUADDR
        lda #$00
        sta PPUADDR
        ldx #$00
copy_palette:
        lda palette,x
        sta PPUDATA
        inx
        cpx #32
        bne copy_palette

        ; The two nametables. With the cartridge wired for vertical
        ; mirroring they sit side by side, which is what scrolling
        ; sideways through 512 pixels of background needs.
        lda #$20
        sta PPUADDR
        lda #$00
        sta PPUADDR
        jsr fill_nametable
        lda #$24
        sta PPUADDR
        lda #$00
        sta PPUADDR
        jsr fill_nametable

        ; The stars, the clouds and the gems, one tile at a time over
        ; the rows that were just filled. Each entry is an address and a
        ; tile; a zero high byte ends the table, and no nametable
        ; address has one.
        ldx #$00
decorate:
        lda decor,x
        beq decorated
        sta PPUADDR
        inx
        lda decor,x
        sta PPUADDR
        inx
        lda decor,x
        sta PPUDATA
        inx
        jmp decorate
decorated:

        ; The sprites into OAM, the scroll to the top left, and on.
        lda #$00
        sta OAMADDR
        lda #OAM_PAGE
        sta OAMDMA

        bit PPUSTATUS
        lda #$00
        sta PPUSCROLL
        sta PPUSCROLL

        lda #CTRL_ON
        sta PPUCTRL
        lda #MASK_ON
        sta PPUMASK

; Everything from here is the NMI's.
idle:
        jmp idle

; ---------------------------------------------------------------------
; fill_nametable — 960 tiles and 64 attribute bytes through $2007.
;
; The caller has pointed $2006 at the top left of a nametable. Each of
; the thirty rows is one tile repeated across the 32 columns, taken from
; `rows`; the eight stores in the inner loop are written out because a
; loop around one of them would cost eight times as much and this runs
; 1920 times.
; ---------------------------------------------------------------------
fill_nametable:
        ldx #$00
fill_row:
        lda rows,x
        ldy #$04
fill_eight:
        sta PPUDATA
        sta PPUDATA
        sta PPUDATA
        sta PPUDATA
        sta PPUDATA
        sta PPUDATA
        sta PPUDATA
        sta PPUDATA
        dey
        bne fill_eight
        inx
        cpx #30
        bne fill_row

        ; The attribute table is the 64 bytes after the 960, and each of
        ; its bytes colours four by four tiles in four quarters.
        ldy #$00
fill_attr:
        lda attributes,y
        sta PPUDATA
        iny
        cpy #64
        bne fill_attr
        rts

; ---------------------------------------------------------------------
; The frame, in vblank
; ---------------------------------------------------------------------
nmi:
        pha
        txa
        pha
        tya
        pha

        ; Reading $2002 takes the vblank flag down, which drops the NMI
        ; line, and puts the write latch back to its first half.
        bit PPUSTATUS

        inc frame

        ; Each of the four sprites one pixel to the right. X walks the
        ; fourth byte of each entry: $0203, $0207, $020B, $020F.
        ldx #$03
move_sprites:
        inc oam,x
        txa
        clc
        adc #$04
        tax
        cpx #$13
        bne move_sprites

        ; Into OAM. This stops the processor for 513 cycles, which is
        ; most of what vblank is for.
        lda #$00
        sta OAMADDR
        lda #OAM_PAGE
        sta OAMDMA

        ; The stars change colour every eighth frame, through eight of
        ; the palette's bright hues.
        lda #$3f
        sta PPUADDR
        lda #$02
        sta PPUADDR
        lda frame
        lsr a
        lsr a
        lsr a
        and #$07
        clc
        adc #$21
        sta PPUDATA

        ; The scroll. $2006 has just been written twice, so the latch is
        ; back at its first half either way, but $2002 is read again
        ; because a program that leaves that to luck is a program that
        ; scrolls sideways one frame in two.
        bit PPUSTATUS
        lda frame
        sta PPUSCROLL
        lda #$00
        sta PPUSCROLL

        ; $2006 wrote the nametable bits of `t` on its way past $3F02;
        ; $2000 is what puts them back, so it goes last.
        lda #CTRL_ON
        sta PPUCTRL
        lda #MASK_ON
        sta PPUMASK

        pla
        tay
        pla
        tax
        pla
        rti

; Nothing in this console raises an IRQ and the reset sets I, but the
; vector has to point at an instruction rather than at whatever $00 is.
irq:
        rti

; ---------------------------------------------------------------------
; Data
; ---------------------------------------------------------------------

; The four palettes for the background and the four for the sprites, in
; the order $3F00 takes them. Entries 16, 20, 24 and 28 are not entries
; at all — $3F10 is another way of writing $3F00 — so they are written
; with the same backdrop as 0, 4, 8 and 12 and nothing moves.
palette:
        .byte $0f, $11, $21, $30      ; background 0: night sky, stars, cloud
        .byte $0f, $19, $2a, $30      ; background 1: the gems and the grass
        .byte $0f, $07, $17, $27      ; background 2: soil and its stones
        .byte $0f, $06, $16, $26      ; background 3: the brick at the bottom
        .byte $0f, $12, $22, $32      ; sprite 0: the blue creature
        .byte $0f, $16, $26, $36      ; sprite 1: a red ball
        .byte $0f, $1a, $2a, $3a      ; sprite 2: a green creature
        .byte $0f, $13, $23, $33      ; sprite 3: a violet ball

; Y, tile, attribute, X. The attribute is palette in bits 1-0, "behind
; the background" in bit 5, horizontal flip in bit 6 and vertical flip
; in bit 7. Sprite zero sits over the soil on purpose: that is where the
; sprite zero hit the picture unit reports comes from.
sprites:
        .byte $98, $01, $00, $20      ; 0: the creature, in front
        .byte $a0, $02, $01, $50      ; 1: a red ball
        .byte $88, $01, $42, $80      ; 2: the creature mirrored, green
        .byte $b0, $02, $a3, $b0      ; 3: a ball behind the background

; One tile per row of the background, top to bottom: sixteen rows of
; sky, the row the gems stand on, the grass line, six of soil and five
; of brick.
rows:
        .byte $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $03, $04, $04, $04, $04, $04
        .byte $04, $02, $02, $02, $02, $02

; The attribute table: eight bytes a row, each byte four quarters of two
; bits, each quarter two tiles by two. The first four rows are all
; palette 0, because they are all sky; the fifth straddles the grass
; line, so its top half is palette 1 and its bottom half palette 2; then
; soil, then the brick in palette 3.
attributes:
        .byte $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00
        .byte $00, $00, $00, $00, $00, $00, $00, $00
        .byte $a5, $a5, $a5, $a5, $a5, $a5, $a5, $a5
        .byte $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa
        .byte $aa, $aa, $aa, $aa, $aa, $aa, $aa, $aa
        .byte $ff, $ff, $ff, $ff, $ff, $ff, $ff, $ff

; Address high, address low, tile. The left nametable first, then the
; right one, which is decorated differently so that the scroll is
; obvious when it brings it into view. Ended by a zero high byte.
decor:
        ; the left nametable, $2000
        .byte $20, $26, $01           ; a star at column 6, row 1
        .byte $20, $5b, $01           ; column 27, row 2
        .byte $20, $7e, $01           ; column 30, row 3
        .byte $20, $83, $06           ; a cloud at columns 3-4, row 4
        .byte $20, $84, $07
        .byte $20, $ae, $01           ; column 14, row 5
        .byte $20, $f4, $06           ; a cloud at columns 20-21, row 7
        .byte $20, $f5, $07
        .byte $21, $02, $01           ; column 2, row 8
        .byte $21, $38, $01           ; column 24, row 9
        .byte $21, $6a, $06           ; a cloud at columns 10-11, row 11
        .byte $21, $6b, $07
        .byte $21, $88, $01           ; column 8, row 12
        .byte $21, $b2, $01           ; column 18, row 13
        .byte $22, $25, $05           ; gems along row 17
        .byte $22, $2d, $05
        .byte $22, $35, $05
        .byte $22, $3d, $05
        ; the right nametable, $2400
        .byte $24, $34, $01           ; column 20, row 1
        .byte $24, $49, $01           ; column 9, row 2
        .byte $24, $62, $01           ; column 2, row 3
        .byte $24, $99, $06           ; a cloud at columns 25-26, row 4
        .byte $24, $9a, $07
        .byte $24, $a6, $01           ; column 6, row 5
        .byte $24, $eb, $06           ; a cloud at columns 11-12, row 7
        .byte $24, $ec, $07
        .byte $25, $1d, $01           ; column 29, row 8
        .byte $25, $2f, $01           ; column 15, row 9
        .byte $25, $63, $06           ; a cloud at columns 3-4, row 11
        .byte $25, $64, $07
        .byte $25, $96, $01           ; column 22, row 12
        .byte $25, $a7, $01           ; column 7, row 13
        .byte $26, $21, $05           ; gems along row 17
        .byte $26, $29, $05
        .byte $26, $31, $05
        .byte $26, $39, $05
        .byte $00                     ; the end

; ---------------------------------------------------------------------
; The three vectors, the last six bytes of the cartridge and therefore
; of the address space. $FFFC is read before any RAM can have been
; written, so this is where a 6502 starts whatever else is true.
; ---------------------------------------------------------------------
        .org $FFFA
        .word nmi                     ; $FFFA  NMI, once a frame
        .word reset                   ; $FFFC  RES
        .word irq                     ; $FFFE  IRQ and BRK
