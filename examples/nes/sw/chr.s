; chr.s — the pattern memory of the demo cartridge: our own tiles.
;
; An NES tile is eight by eight pixels, two bits a pixel, stored as two
; bitplanes of eight bytes: the first eight bytes are bit 0 of each
; pixel, row by row, and the next eight are bit 1. Bit 7 of a byte is the
; leftmost pixel of its row. The two bits are an index into one of the
; four-colour palettes, and index 0 is *transparent* — the backdrop shows
; through it for a background tile and the background shows through it
; for a sprite.
;
; Every tile below is drawn twice: once as art in the comment above it,
; where `.` is index 0 and `1`, `2` and `3` are the three colours of
; whichever palette it is drawn with, and once as the sixteen bytes. The
; two are checked against each other by `the_tiles_are_the_art_drawn_
; above_them` in tests/nes.rs, which reads the comment, works out what
; the sixteen bytes have to be, and compares — so a typo in either one is
; a failing test rather than a wrong pixel nobody notices.
;
; Nothing here is anyone else's artwork: these are eleven tiles drawn for
; this example.
;
; chr.hex is this file assembled. tests/nes.rs assembles it again and
; fails if the two differ; run it with UPDATE_EXPECT=1 to rewrite the hex
; after editing this file.

; ---------------------------------------------------------------------
; Pattern table 0, at $0000. $2000 bit 4 clear points the background at
; it, which is what sw/demo.s does.
; ---------------------------------------------------------------------
        .org $0000

; tile $00 — nothing at all: the colour the sky is painted with
;   ........
;   ........
;   ........
;   ........
;   ........
;   ........
;   ........
;   ........
tile_sky:
        .byte $00, $00, $00, $00, $00, $00, $00, $00   ; plane 0
        .byte $00, $00, $00, $00, $00, $00, $00, $00   ; plane 1

; tile $01 — a four-pointed star, bright in the middle
;   ........
;   ...11...
;   ...11...
;   .112211.
;   .112211.
;   ...11...
;   ...11...
;   ........
tile_star:
        .byte $00, $18, $18, $66, $66, $18, $18, $00   ; plane 0
        .byte $00, $00, $00, $18, $18, $00, $00, $00   ; plane 1

; tile $02 — two courses of brick, the joints offset
;   11111111
;   22222212
;   22222212
;   22222212
;   11111111
;   21222222
;   21222222
;   21222222
tile_brick:
        .byte $ff, $02, $02, $02, $ff, $40, $40, $40   ; plane 0
        .byte $00, $fd, $fd, $fd, $00, $bf, $bf, $bf   ; plane 1

; tile $03 — the top of the ground: grass over soil
;   33333333
;   33333333
;   23232323
;   22222222
;   22222222
;   22122122
;   22222222
;   22222222
tile_grass:
        .byte $ff, $ff, $55, $00, $00, $24, $00, $00   ; plane 0
        .byte $ff, $ff, $ff, $ff, $ff, $db, $ff, $ff   ; plane 1

; tile $04 — soil with a few stones in it
;   22222222
;   22122222
;   22222222
;   22222122
;   22222222
;   21222222
;   22222222
;   22222212
tile_dirt:
        .byte $00, $20, $00, $04, $00, $40, $00, $02   ; plane 0
        .byte $ff, $df, $ff, $fb, $ff, $bf, $ff, $fd   ; plane 1

; tile $05 — a cut stone, lit from the left
;   ...33...
;   ..3223..
;   .322223.
;   32222223
;   32222223
;   .322223.
;   ..3223..
;   ...33...
tile_gem:
        .byte $18, $24, $42, $81, $81, $42, $24, $18   ; plane 0
        .byte $18, $3c, $7e, $ff, $ff, $7e, $3c, $18   ; plane 1

; tile $06 — the left half of a cloud
;   ........
;   ....111.
;   ..11111.
;   .1111111
;   11111111
;   11111111
;   .111111.
;   ........
tile_cloud_l:
        .byte $00, $0e, $3e, $7f, $ff, $ff, $7e, $00   ; plane 0
        .byte $00, $00, $00, $00, $00, $00, $00, $00   ; plane 1

; tile $07 — and its right half, which joins it
;   ........
;   .111....
;   .11111..
;   1111111.
;   11111111
;   11111111
;   .111111.
;   ........
tile_cloud_r:
        .byte $00, $70, $7c, $fe, $ff, $ff, $7e, $00   ; plane 0
        .byte $00, $00, $00, $00, $00, $00, $00, $00   ; plane 1

; ---------------------------------------------------------------------
; Pattern table 1, at $1000. $2000 bit 3 set points the sprites at it.
; Tile $00 has to exist and has to be blank: a slot with no sprite in it
; still fetches something, and what it fetches is tile zero.
; ---------------------------------------------------------------------
        .org $1000

; tile $00 — nothing: the tile an unused slot fetches
;   ........
;   ........
;   ........
;   ........
;   ........
;   ........
;   ........
;   ........
tile_sp_blank:
        .byte $00, $00, $00, $00, $00, $00, $00, $00   ; plane 0
        .byte $00, $00, $00, $00, $00, $00, $00, $00   ; plane 1

; tile $01 — a round creature with two eyes and two feet
;   ..2222..
;   .222222.
;   22322232
;   22322232
;   22222222
;   22233222
;   .222222.
;   ..2..2..
tile_sp_bug:
        .byte $00, $00, $22, $22, $00, $18, $00, $00   ; plane 0
        .byte $3c, $7e, $ff, $ff, $ff, $ff, $7e, $24   ; plane 1

; tile $02 — a plain ball, for the sprites that only move
;   ........
;   ..3333..
;   .333333.
;   .333333.
;   .333333.
;   .333333.
;   ..3333..
;   ........
tile_sp_ball:
        .byte $00, $3c, $7e, $7e, $7e, $7e, $3c, $00   ; plane 0
        .byte $00, $3c, $7e, $7e, $7e, $7e, $3c, $00   ; plane 1
