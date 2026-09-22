; hello.s: print a line over the computer's UART, then halt.
;
; The memory map is the one rtl/computer_top.v decodes:
;
;   $0000-$03FF  RAM   1 KiB: zero page, the stack in page one, the rest
;   $D000        UART  DATA   write a byte to send it
;   $D001        UART  STATUS bit 0: the transmitter can take a byte
;   $F800-$FFFF  ROM   2 KiB: this program, and the vectors at its top
;
; There is no monitor, no operating system and no loader: the part reads
; $FFFC when reset is released and starts there, so the program *is* the
; reset vector's target. It sets up the stack itself, because on a 6502 a
; reset leaves the stack pointer three lower than wherever it happened to
; be and nothing else is defined.
;
; hello.hex is this file assembled. tests/mos6502_computer.rs assembles it
; again and fails if the two differ; run it with UPDATE_EXPECT=1 to
; rewrite the hex after editing this file.

        .equ UART_DATA,   $D000
        .equ UART_STATUS, $D001
        .equ TX_READY,    $01

        .org $F800

; The RES sequence lands here, seven cycles after reset is released.
reset:
        sei                     ; nothing raises an interrupt, but say so
        cld                     ; binary arithmetic: every 6502 program
                                ; starts by clearing D, because RES does
                                ; not define it on the real part
        ldx #$ff
        txs                     ; the stack at $01FF, growing down page one
        ldy #$00
print:
        lda message,y           ; absolute,Y: the string is in ROM
        beq halt                ; the NUL ends it
        jsr putc
        iny
        bne print               ; 255 characters is plenty
halt:
        jmp halt                ; nothing more to do: spin here

; putc(A = byte): wait until the transmitter can take a byte, then hand
; it over. The byte goes on the stack while STATUS is polled, because the
; 6502 has one accumulator and the poll needs it; Y is untouched, which
; is what lets the caller keep its index in it.
putc:
        pha
wait:
        lda UART_STATUS
        and #TX_READY
        beq wait
        pla
        sta UART_DATA
        rts

; Nothing in this system raises an interrupt and I is set above, but the
; vectors have to point at instructions rather than at whatever $00 is.
nmi:
irq:
        rti

message:
        .string "Hello from Reticle\n"

; The three vectors, at the top of the ROM and therefore at the top of
; the address space. This is the part of a 6502 memory map that is not
; negotiable: $FFFC is read before any RAM can have been written.
        .org $FFFA
        .word nmi               ; $FFFA  NMI
        .word reset             ; $FFFC  RES
        .word irq               ; $FFFE  IRQ and BRK
