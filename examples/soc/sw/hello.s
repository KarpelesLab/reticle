# hello.s: print a line over the SoC's UART, then halt.
#
# The memory map is the one rtl/soc_top.v decodes:
#
#   0x0000_0000  ROM   this program and its string, read by both ports
#   0x0001_0000  RAM   1 KiB of data; the stack grows down from its top
#   0x0002_0000  UART  +0 TXDATA (write a byte to send it)
#                      +4 STATUS (bit 0: the transmitter can take a byte)
#
# There is no C runtime and no linker: the program starts at address 0,
# which is the core's reset vector. Every register is written before it
# is read, because RV32I leaves x1..x31 undefined after reset.
#
# hello.hex is this file assembled. tests/soc.rs assembles it again and
# fails if the two differ; run it with UPDATE_EXPECT=1 to rewrite the hex
# after editing this file.

        .equ RAM_TOP,   0x10400     # one past the last byte of RAM
        .equ UART_HI,   0x20        # lui immediate for 0x0002_0000
        .equ TXDATA,    0
        .equ STATUS,    4
        .equ TX_READY,  1

_start:
        lui   sp, 0x10              # sp = 0x0001_0000, the RAM base ...
        addi  sp, sp, 0x400         # ... plus its size: the stack top
        lui   s0, UART_HI           # s0 = the UART's registers
        addi  a0, zero, message     # a0 = the string (it sits low in ROM)
        jal   ra, puts
halt:
        j     halt                  # nothing more to do: spin here

# puts(a0 = string): send every byte up to the terminating NUL.
# It calls putc, so it keeps its own return address on the stack.
puts:
        addi  sp, sp, -4
        sw    ra, 0(sp)
        addi  a1, a0, 0             # a1 walks the string; putc leaves it
puts_next:
        lbu   a0, 0(a1)
        beq   a0, zero, puts_done
        jal   ra, putc
        addi  a1, a1, 1
        j     puts_next
puts_done:
        lw    ra, 0(sp)
        addi  sp, sp, 4
        ret

# putc(a0 = byte): wait until the transmitter can take a byte, then
# hand it over. Clobbers t0.
putc:
        lw    t0, STATUS(s0)
        andi  t0, t0, TX_READY
        beq   t0, zero, putc
        sb    a0, TXDATA(s0)
        ret

message:
        .string "Hello from Reticle\n"
