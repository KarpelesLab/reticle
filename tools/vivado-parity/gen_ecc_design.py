#!/usr/bin/env python3
"""Writes a design that sets as many configuration bits as it can.

    gen_ecc_design.py <top> <seed> <luts> <brams> > <top>.v

The frame ECC is a function of a frame's bits, so what proves it is frames with
many bits set, in every word: random lookup-table contents across the die and
block RAMs full of random data. It computes nothing of interest. The frames
Vivado wrote for it are where `testdata/fpga/vivado/bitstream/ecc_frames.txt`
comes from.
"""
import random
import sys


def main():
    top, seed, luts, brams = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
    r = random.Random(seed)
    ffs = max(luts // 4, 64)
    out = [f"module {top} (input wire clk, input wire si, output wire so);"]
    out.append(f"  wire [{ffs - 1}:0] q;")
    out.append(f"  wire [{luts - 1}:0] l;")
    out.append(f"  wire [{max(brams, 1) - 1}:0] m;")
    out.append("  assign so = q[0];")
    for i in range(luts):
        pins = ", ".join(
            f".I{k}(q[{r.randrange(ffs)}])" if not (i == 0 and k == 0) else f".I{k}(si)"
            for k in range(6)
        )
        init = r.getrandbits(64)
        out.append(
            f"  (* DONT_TOUCH = \"TRUE\" *) LUT6 #(.INIT(64'h{init:016x})) "
            f"u{i} ({pins}, .O(l[{i}]));"
        )
    for i in range(ffs):
        d = f"l[{i}]"
        if i < brams:
            d = f"l[{i}] ^ m[{i}]"
        extra = f"l[{i + ffs}]" if i + ffs < luts else "1'b0"
        init = r.getrandbits(1)
        out.append(
            f"  FDRE #(.INIT(1'b{init})) f{i} (.C(clk), .CE(1'b1), .R(1'b0), "
            f".D({d} ^ {extra}), .Q(q[{i}]));"
        )
    # Each table past the first `ffs` feeds a flip-flop through an XOR above, so
    # only the rest would be dead; DONT_TOUCH keeps those too.
    for i in range(brams):
        inits = ", ".join(f".INIT_{k:02X}(256'h{r.getrandbits(256):064x})" for k in range(128))
        initp = ", ".join(f".INITP_{k:02X}(256'h{r.getrandbits(256):064x})" for k in range(16))
        addr = ", ".join(f"q[{r.randrange(ffs)}]" for _ in range(10))
        out.append(
            f"  wire [31:0] do{i};\n"
            f"  RAMB36E1 #(.READ_WIDTH_A(36), .WRITE_WIDTH_A(36), {inits}, {initp}) b{i} (\n"
            f"    .CLKARDCLK(clk), .CLKBWRCLK(clk), .ENARDEN(1'b1), .ENBWREN(1'b0),\n"
            f"    .RSTRAMARSTRAM(1'b0), .RSTREGARSTREG(1'b0), .RSTRAMB(1'b0), .RSTREGB(1'b0),\n"
            f"    .REGCEAREGCE(1'b1), .REGCEB(1'b0), .WEA(4'b0), .WEBWE(8'b0),\n"
            f"    .ADDRARDADDR({{1'b1, {addr}, 5'b0}}), .ADDRBWRADDR(16'h8000),\n"
            f"    .DIADI(32'b0), .DIBDI(32'b0), .DIPADIP(4'b0), .DIPBDIP(4'b0),\n"
            f"    .CASCADEINA(1'b0), .CASCADEINB(1'b0), .INJECTDBITERR(1'b0), .INJECTSBITERR(1'b0),\n"
            f"    .DOADO(do{i}));\n"
            f"  assign m[{i}] = do{i}[0];"
        )
    out.append("endmodule")
    print("\n".join(out))


if __name__ == "__main__":
    main()
