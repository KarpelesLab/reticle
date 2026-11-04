#!/usr/bin/env python3
"""Small multiplies, to ask Vivado what it makes of each.

One design per question: which registers go into the block, which widths fit,
what a wider one is split into. Each takes its operands from a shift register
behind one pin and gives one parity pin, so that pins do not limit it and
nothing is folded away. Run them with `run_all.sh`; the answers that are
committed are under testdata/fpga/vivado/dsp/inference.txt.

    tools/vivado-parity/gen_dsp_inference.py <dir>
"""
import os
import sys


def ty(signed, width):
    return f"{'signed ' if signed else ''}[{width - 1}:0]"


def design(name, a, b, signed, areg=0, breg=0, mreg=0, ctrl="", result=None, comment=""):
    """`a` x `b`, with `areg` registers on `a`, `breg` on `b`, `mreg` after.

    `ctrl` picks how the registers are controlled:
      ""     plain, "en" clock enable, "rst" synchronous reset, "arst"
      asynchronous reset, "rst1" synchronous reset to one, "init" an initial
      value, "fan" every register also read elsewhere.
    """
    result = result or a + b
    n = a + b + 4
    lines = [f"// {comment or name}", f"module {name} (input wire clk, input wire si, output reg so);"]
    lines.append(f"  reg [{n - 1}:0] in;")
    lines.append(f"  wire {ty(signed, a)} a0 = in[{a - 1}:0];")
    lines.append(f"  wire {ty(signed, b)} b0 = in[{a + b - 1}:{a}];")
    edge = "posedge clk"
    head = f"if (in[{n - 1}])" if ctrl == "en" else ""
    rst = "rr"
    lines.append(f"  wire rr = in[{n - 2}];")
    if ctrl == "arst":
        edge = f"posedge clk or posedge {rst}"
    def stage(q, d, width, sign):
        init = " = 1" if ctrl == "init" else ""
        lines.append(f"  reg {ty(sign, width)} {q}{init};")
        if ctrl in ("rst", "arst"):
            return f"    if ({rst}) {q} <= 0; else {head} {q} <= {d};"
        if ctrl == "rst1":
            return f"    if ({rst}) {q} <= 1; else {head} {q} <= {d};"
        return f"    {head} {q} <= {d};"
    body = []
    prev = "a0"
    for i in range(areg):
        body.append(stage(f"a{i + 1}", prev, a, signed))
        prev = f"a{i + 1}"
    aq = prev
    prev = "b0"
    for i in range(breg):
        body.append(stage(f"b{i + 1}", prev, b, signed))
        prev = f"b{i + 1}"
    bq = prev
    lines.append(f"  wire {ty(signed, result)} m0 = {aq} * {bq};")
    prev = "m0"
    for i in range(mreg):
        body.append(stage(f"m{i + 1}", prev, result, signed))
        prev = f"m{i + 1}"
    out = f"^{prev}"
    if ctrl == "fan":
        out += f" ^ ^{aq} ^ ^{bq}"
    lines.append(f"  always @(posedge clk) begin in <= {{in[{n - 2}:0], si}}; so <= {out}; end")
    if body:
        lines.append(f"  always @({edge}) begin")
        lines.extend(body)
        lines.append("  end")
    lines.append("endmodule")
    return "\n".join(lines) + "\n"


CASES = [
    # What each register position becomes, on an 18x18.
    ("r_none", dict(a=18, b=18, signed=True)),
    ("r_a1", dict(a=18, b=18, signed=True, areg=1, breg=1)),
    ("r_a2", dict(a=18, b=18, signed=True, areg=2, breg=2)),
    ("r_a3", dict(a=18, b=18, signed=True, areg=3, breg=3)),
    ("r_aonly", dict(a=18, b=18, signed=True, areg=1)),
    ("r_m1", dict(a=18, b=18, signed=True, mreg=1)),
    ("r_m2", dict(a=18, b=18, signed=True, mreg=2)),
    ("r_m3", dict(a=18, b=18, signed=True, mreg=3)),
    ("r_a1m1", dict(a=18, b=18, signed=True, areg=1, breg=1, mreg=1)),
    ("r_a1m2", dict(a=18, b=18, signed=True, areg=1, breg=1, mreg=2)),
    ("r_a2m2", dict(a=18, b=18, signed=True, areg=2, breg=2, mreg=2)),
    # Register controls.
    ("c_en", dict(a=18, b=18, signed=True, areg=1, breg=1, mreg=2, ctrl="en")),
    ("c_rst", dict(a=18, b=18, signed=True, areg=1, breg=1, mreg=2, ctrl="rst")),
    ("c_arst", dict(a=18, b=18, signed=True, areg=1, breg=1, mreg=2, ctrl="arst")),
    ("c_rst1", dict(a=18, b=18, signed=True, areg=1, breg=1, mreg=2, ctrl="rst1")),
    ("c_init", dict(a=18, b=18, signed=True, areg=1, breg=1, mreg=2, ctrl="init")),
    ("c_fan", dict(a=18, b=18, signed=True, areg=1, breg=1, mreg=2, ctrl="fan")),
    # Widths, signed.
    ("s_8x8", dict(a=8, b=8, signed=True)),
    ("s_16x16", dict(a=16, b=16, signed=True, mreg=1)),
    ("s_25x18", dict(a=25, b=18, signed=True, mreg=1)),
    ("s_18x25", dict(a=18, b=25, signed=True, mreg=1)),
    ("s_26x18", dict(a=26, b=18, signed=True, mreg=1)),
    ("s_25x19", dict(a=25, b=19, signed=True, mreg=1)),
    ("s_26x26", dict(a=26, b=26, signed=True, mreg=1)),
    ("s_32x32", dict(a=32, b=32, signed=True, mreg=1)),
    ("s_32x32t", dict(a=32, b=32, signed=True, mreg=1, result=32)),
    ("s_35x35", dict(a=35, b=35, signed=True, mreg=1)),
    ("s_48x48", dict(a=48, b=48, signed=True, mreg=1)),
    # Widths, unsigned.
    ("u_8x8", dict(a=8, b=8, signed=False)),
    ("u_17x17", dict(a=17, b=17, signed=False, mreg=1)),
    ("u_18x18", dict(a=18, b=18, signed=False, mreg=1)),
    ("u_24x17", dict(a=24, b=17, signed=False, mreg=1)),
    ("u_25x18", dict(a=25, b=18, signed=False, mreg=1)),
    ("u_32x32", dict(a=32, b=32, signed=False, mreg=1)),
    ("u_32x32t", dict(a=32, b=32, signed=False, mreg=1, result=32)),
    # Small multiplies Vivado may leave in logic.
    ("s_3x3", dict(a=3, b=3, signed=True)),
    ("u_4x4", dict(a=4, b=4, signed=False)),
    ("u_6x6", dict(a=6, b=6, signed=False)),
    ("s_4x12", dict(a=4, b=12, signed=True)),
    ("u_8x3", dict(a=8, b=3, signed=False)),
]

# Where a multiply stops being worth a block.
CASES += [(f"t_s{a}x{b}", dict(a=a, b=b, signed=True)) for a, b in (
    (9, 9), (10, 10), (11, 11), (12, 12), (13, 13), (14, 14), (15, 15),
    (2, 18), (3, 18), (4, 18), (5, 18), (6, 18), (7, 18), (8, 18), (9, 18), (10, 18), (5, 10), (5, 12), (5, 16), (6, 10), (6, 12), (7, 12), (8, 12), (8, 10), (9, 10), (4, 16), (3, 25),
    (6, 11), (7, 11), (7, 10), (8, 11), (5, 25), (6, 25),
)]
CASES += [(f"t_u{a}x{b}", dict(a=a, b=b, signed=False)) for a, b in (
    (9, 9), (10, 10), (11, 11), (12, 12), (13, 13), (14, 14), (15, 15), (16, 16), (8, 12), (6, 16), (4, 17),
)]
CASES += [(f"t_r{a}x{b}", dict(a=a, b=b, signed=True, areg=1, breg=1, mreg=2)) for a, b in (
    (4, 4), (6, 6), (8, 8), (9, 9), (10, 10), (11, 11), (12, 12),
)]

XDC = (
    "create_clock -period 5 -name clk [get_ports clk]\n"
    "set_property PACKAGE_PIN N11 [get_ports clk]\n"
    "set_property PACKAGE_PIN L4 [get_ports si]\n"
    "set_property PACKAGE_PIN L5 [get_ports so]\n"
    "set_property IOSTANDARD LVCMOS33 [get_ports *]\n"
)

# Multiplies whose shape is not `design`'s.
EXTRA = {
    "k_const": """// By a constant: which operand goes on B.
module k_const (input wire clk, input wire si, output reg so);
  reg [17:0] in;
  reg signed [35:0] m;
  always @(posedge clk) begin
    in <= {in[16:0], si};
    m <= $signed(in) * 18'sd5021;
    so <= ^m;
  end
endmodule
""",
    "k_pow2": """// By a power of two, which is a shift.
module k_pow2 (input wire clk, input wire si, output reg so);
  reg [17:0] in;
  reg signed [35:0] m;
  always @(posedge clk) begin
    in <= {in[16:0], si};
    m <= $signed(in) * 18'sd1024;
    so <= ^m;
  end
endmodule
""",
    "k_small": """// By a small constant.
module k_small (input wire clk, input wire si, output reg so);
  reg [17:0] in;
  reg signed [35:0] m;
  always @(posedge clk) begin
    in <= {in[16:0], si};
    m <= $signed(in) * 18'sd5;
    so <= ^m;
  end
endmodule
""",
    "f_two": """// One product read twice.
module f_two (input wire clk, input wire si, output reg so);
  reg [35:0] in;
  reg signed [17:0] a, b;
  reg signed [35:0] p, q;
  wire signed [35:0] m = a * b;
  always @(posedge clk) begin
    in <= {in[34:0], si};
    a <= in[17:0];
    b <= in[35:18];
    p <= m;
    q <= m + p;
    so <= ^q;
  end
endmodule
""",
    "f_macc": """// A multiply and accumulate.
module f_macc (input wire clk, input wire si, output reg so);
  reg [35:0] in;
  reg signed [17:0] a, b;
  reg signed [47:0] acc;
  always @(posedge clk) begin
    in <= {in[34:0], si};
    a <= in[17:0];
    b <= in[35:18];
    acc <= acc + a * b;
    so <= ^acc;
  end
endmodule
""",
    "f_add": """// A multiply and add of another value.
module f_add (input wire clk, input wire si, output reg so);
  reg [47:0] in;
  reg signed [17:0] a, b;
  reg signed [11:0] c;
  reg signed [36:0] p;
  always @(posedge clk) begin
    in <= {in[46:0], si};
    a <= in[17:0];
    b <= in[35:18];
    c <= in[47:36];
    p <= a * b + c;
    so <= ^p;
  end
endmodule
""",
    "f_preadd": """// A pre-adder.
module f_preadd (input wire clk, input wire si, output reg so);
  reg [59:0] in;
  reg signed [23:0] a, d;
  reg signed [17:0] b;
  reg signed [42:0] p;
  always @(posedge clk) begin
    in <= {in[58:0], si};
    a <= in[23:0];
    d <= in[47:24];
    b <= in[59:48];
    p <= (a + d) * b;
    so <= ^p;
  end
endmodule
""",
    "f_square": """// One operand twice.
module f_square (input wire clk, input wire si, output reg so);
  reg [17:0] in;
  reg signed [17:0] a;
  reg signed [35:0] p;
  always @(posedge clk) begin
    in <= {in[16:0], si};
    a <= in;
    p <= a * a;
    so <= ^p;
  end
endmodule
""",
}


def long_sum(name, terms):
    """A sum of products longer than a column has blocks in one clock region."""
    lines = [f"// A sum of {terms} products: a cascade that has to cross a clock row.",
             f"module {name} (input wire clk, input wire si, output reg so);",
             f"  reg [{terms + 16}:0] in;", "  reg signed [47:0] s;"]
    for k in range(terms):
        lines.append(f"  reg signed [15:0] t{k}; reg signed [31:0] m{k};")
    lines.append(f"  always @(posedge clk) begin in <= {{in[{terms + 15}:0], si}};")
    for k in range(terms):
        lines.append(f"    t{k} <= in[{k + 15}:{k}]; m{k} <= t{k} * {1000 + 37 * k};")
    lines.append("    s <= " + " + ".join(f"m{k}" for k in range(terms)) + ";")
    lines.append("    so <= ^s;\n  end\nendmodule\n")
    return "\n".join(lines)


EXTRA["l_sum30"] = long_sum("l_sum30", 30)

EXTRA["n_pair"] = """// Two blocks by name, joined by every cascade there is.
module n_pair (input wire clk, input wire si, output reg so);
  reg [47:0] in;
  wire [29:0] ac;
  wire [17:0] bc;
  wire [47:0] pc;
  wire cc, ms;
  wire [47:0] p;
  DSP48E1 #(.AREG(1), .ACASCREG(1), .BREG(1), .BCASCREG(1), .CREG(0), .ADREG(0), .DREG(0), .MREG(1), .PREG(1),
            .INMODEREG(0), .OPMODEREG(0), .ALUMODEREG(0), .CARRYINREG(0), .CARRYINSELREG(0))
    low (.CLK(clk), .A({{12{in[17]}}, in[17:0]}), .B(in[35:18]), .C(48'd0), .D(25'd0), .CARRYIN(1'b0),
         .CARRYINSEL(3'd0), .INMODE(5'd0), .OPMODE(7'b0000101), .ALUMODE(4'd0),
         .CEA1(1'b0), .CEA2(1'b1), .CEB1(1'b0), .CEB2(1'b1), .CEM(1'b1), .CEP(1'b1), .CEC(1'b0), .CEAD(1'b0), .CED(1'b0),
         .CECTRL(1'b0), .CEALUMODE(1'b0), .CECARRYIN(1'b0), .CEINMODE(1'b0),
         .RSTA(1'b0), .RSTB(1'b0), .RSTC(1'b0), .RSTD(1'b0), .RSTM(1'b0), .RSTP(1'b0), .RSTCTRL(1'b0),
         .RSTALLCARRYIN(1'b0), .RSTALUMODE(1'b0), .RSTINMODE(1'b0),
         .ACOUT(ac), .BCOUT(bc), .PCOUT(pc), .CARRYCASCOUT(cc), .MULTSIGNOUT(ms));
  DSP48E1 #(.AREG(1), .ACASCREG(1), .BREG(1), .BCASCREG(1), .CREG(0), .ADREG(0), .DREG(0), .MREG(1), .PREG(1),
            .INMODEREG(0), .OPMODEREG(0), .ALUMODEREG(0), .CARRYINREG(0), .CARRYINSELREG(0),
            .A_INPUT("CASCADE"), .B_INPUT("CASCADE"))
    high (.CLK(clk), .A(30'd0), .B(18'd0), .ACIN(ac), .BCIN(bc), .PCIN(pc), .CARRYCASCIN(cc), .MULTSIGNIN(ms),
          .C(48'd0), .D(25'd0), .CARRYIN(1'b0), .CARRYINSEL(3'd0), .INMODE(5'd0), .OPMODE(7'b1010101), .ALUMODE(4'd0),
          .CEA1(1'b0), .CEA2(1'b1), .CEB1(1'b0), .CEB2(1'b1), .CEM(1'b1), .CEP(1'b1), .CEC(1'b0), .CEAD(1'b0), .CED(1'b0),
          .CECTRL(1'b0), .CEALUMODE(1'b0), .CECARRYIN(1'b0), .CEINMODE(1'b0),
          .RSTA(1'b0), .RSTB(1'b0), .RSTC(1'b0), .RSTD(1'b0), .RSTM(1'b0), .RSTP(1'b0), .RSTCTRL(1'b0),
          .RSTALLCARRYIN(1'b0), .RSTALUMODE(1'b0), .RSTINMODE(1'b0), .P(p));
  always @(posedge clk) begin in <= {in[46:0], si}; so <= ^p; end
endmodule
"""


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    only = sys.argv[2:]
    for name, args in CASES:
        if only and not any(name.startswith(prefix) for prefix in only):
            continue
        with open(os.path.join(out, f"{name}.v"), "w") as f:
            f.write(design(name, **args))
    for name, text in EXTRA.items():
        if only and not any(name.startswith(prefix) for prefix in only):
            continue
        with open(os.path.join(out, f"{name}.v"), "w") as f:
            f.write(text)
    for name in [c[0] for c in CASES] + list(EXTRA):
        if only and not any(name.startswith(prefix) for prefix in only):
            continue
        with open(os.path.join(out, f"{name}.xdc"), "w") as f:
            f.write(XDC)


if __name__ == "__main__":
    main()
