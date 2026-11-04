#!/usr/bin/env python3
"""A parameter sweep of one DSP48E1, to ask what each setting changes in its tile.

Writes one design per setting into <dir>: a DSP48E1 at a fixed site, with every
input fed from a shift register so that nothing is folded away, and an output
parity so that nothing is removed. Run them with `run_all.sh <dir> <part> <out>`
and decode each `.bit` with `reticle fpga --device xc7a100t-ftg256 --decode`;
`dsp_sweep_report.py` then writes the measured table under
testdata/fpga/vivado/dsp/.

Only what Vivado answered is committed, not its models.

    tools/vivado-parity/gen_dsp_sweep.py <dir> [setting...]
"""
import os
import sys

# The settings the base has: USE_MULT=MULTIPLY with every register on.
BASE = dict(
    A_INPUT='"DIRECT"', B_INPUT='"DIRECT"', USE_DPORT='"FALSE"', USE_MULT='"MULTIPLY"',
    USE_SIMD='"ONE48"', AUTORESET_PATDET='"NO_RESET"',
    MASK="48'h3fffffffffff", PATTERN="48'h000000000000", SEL_MASK='"MASK"', SEL_PATTERN='"PATTERN"',
    USE_PATTERN_DETECT='"NO_PATDET"', ACASCREG=1, ADREG=1, ALUMODEREG=1, AREG=1, BCASCREG=1, BREG=1,
    CARRYINREG=1, CARRYINSELREG=1, CREG=1, DREG=1, INMODEREG=1, MREG=1, OPMODEREG=1, PREG=1,
)

SWEEP = {
    "base": {},
    # USE_MULT, which the database has no bits for.
    "mult_none": {"USE_MULT": '"NONE"', "MREG": 0},
    "mult_dyn": {"USE_MULT": '"DYNAMIC"'},
    # Each register alone.
    "mreg0": {"MREG": 0},
    "preg0": {"PREG": 0},
    "creg0": {"CREG": 0},
    "opreg0": {"OPMODEREG": 0},
    "alureg0": {"ALUMODEREG": 0},
    "inreg0": {"INMODEREG": 0},
    "ciselreg0": {"CARRYINSELREG": 0},
    "cireg0": {"CARRYINREG": 0},
    # The A and B pipelines, whose features are not one per parameter.
    "areg0": {"AREG": 0, "ACASCREG": 0},
    "areg2": {"AREG": 2, "ACASCREG": 2},
    "areg2c1": {"AREG": 2, "ACASCREG": 1},
    "breg0": {"BREG": 0, "BCASCREG": 0},
    "breg2": {"BREG": 2, "BCASCREG": 2},
    "breg2c1": {"BREG": 2, "BCASCREG": 1},
    # The pre-adder and its port.
    "dport": {"USE_DPORT": '"TRUE"'},
    "dport_dreg0": {"USE_DPORT": '"TRUE"', "DREG": 0},
    "dport_adreg0": {"USE_DPORT": '"TRUE"', "ADREG": 0},
    "dport_both0": {"USE_DPORT": '"TRUE"', "DREG": 0, "ADREG": 0},
    "nodport_dreg0": {"DREG": 0},
    "nodport_adreg0": {"ADREG": 0},
    # SIMD, which needs the multiplier off.
    "simd2": {"USE_SIMD": '"TWO24"', "USE_MULT": '"NONE"', "MREG": 0},
    "simd4": {"USE_SIMD": '"FOUR12"', "USE_MULT": '"NONE"', "MREG": 0},
    # The pattern detector.
    "patdet": {"USE_PATTERN_DETECT": '"PATDET"'},
    "mask0": {"MASK": "48'h000000000000"},
    "pat1": {"PATTERN": "48'h000000000001"},
    "patall": {"PATTERN": "48'hffffffffffff"},
    "maskall": {"MASK": "48'hffffffffffff"},
    "selmask_c": {"SEL_MASK": '"C"'},
    "selmask_r1": {"SEL_MASK": '"ROUNDING_MODE1"'},
    "selmask_r2": {"SEL_MASK": '"ROUNDING_MODE2"'},
    "selpat_c": {"SEL_PATTERN": '"C"'},
    "auto_match": {"AUTORESET_PATDET": '"RESET_MATCH"'},
    "auto_notmatch": {"AUTORESET_PATDET": '"RESET_NOT_MATCH"'},
    # Cascade inputs.
    "acasc": {"A_INPUT": '"CASCADE"'},
    "bcasc": {"B_INPUT": '"CASCADE"'},
    # The inverters, each bit alone.
    "inv_clk": {"IS_CLK_INVERTED": "1'b1"},
    "inv_carryin": {"IS_CARRYIN_INVERTED": "1'b1"},
    "inv_alu0": {"IS_ALUMODE_INVERTED": "4'b0001"},
    "inv_alu3": {"IS_ALUMODE_INVERTED": "4'b1000"},
    "inv_in0": {"IS_INMODE_INVERTED": "5'b00001"},
    "inv_in4": {"IS_INMODE_INVERTED": "5'b10000"},
    "inv_op0": {"IS_OPMODE_INVERTED": "7'b0000001"},
    "inv_op6": {"IS_OPMODE_INVERTED": "7'b1000000"},
    # Other blocks: which site a feature prefix means, and whether the bit the
    # database does not name moves as the named ones do.
    "site_odd": {"@LOC": "DSP48_X2Y51"},
    "site_odd_none": {"@LOC": "DSP48_X2Y51", "USE_MULT": '"NONE"', "MREG": 0},
    "site_r": {"@LOC": "DSP48_X1Y50"},
    "site_r_none": {"@LOC": "DSP48_X1Y50", "USE_MULT": '"NONE"', "MREG": 0},
    "site_r_odd": {"@LOC": "DSP48_X1Y51"},
    "site_r_odd_none": {"@LOC": "DSP48_X1Y51", "USE_MULT": '"NONE"', "MREG": 0},
    # What a constant on a pin is built from. `@TIE` replaces the shift
    # register on those ports.
    "tie_zero": {"@TIE": {p: "0" for p in ("A", "B", "C", "D", "ALUMODE", "CARRYINSEL", "INMODE", "OPMODE", "CARRYIN", "CEA1", "CEA2", "CEAD", "CEALUMODE", "CEB1", "CEB2", "CEC", "CECARRYIN", "CECTRL", "CED", "CEINMODE", "CEM", "CEP", "RSTA", "RSTALLCARRYIN", "RSTALUMODE", "RSTB", "RSTC", "RSTCTRL", "RSTD", "RSTINMODE", "RSTM", "RSTP")}},
    "tie_one": {"@TIE": {p: "1" for p in ("A", "B", "C", "D", "ALUMODE", "CARRYINSEL", "INMODE", "OPMODE", "CARRYIN", "CEA1", "CEA2", "CEAD", "CEALUMODE", "CEB1", "CEB2", "CEC", "CECARRYIN", "CECTRL", "CED", "CEINMODE", "CEM", "CEP", "RSTA", "RSTALLCARRYIN", "RSTALUMODE", "RSTB", "RSTC", "RSTCTRL", "RSTD", "RSTINMODE", "RSTM", "RSTP")}},
    "tie_zero_inv": {
        "IS_CLK_INVERTED": "1'b0", "IS_CARRYIN_INVERTED": "1'b1", "IS_ALUMODE_INVERTED": "4'b1111",
        "IS_INMODE_INVERTED": "5'b11111", "IS_OPMODE_INVERTED": "7'b1111111",
        "@TIE": {p: "0" for p in ("ALUMODE", "INMODE", "OPMODE", "CARRYIN")},
    },
    "tie_one_inv": {
        "IS_CARRYIN_INVERTED": "1'b1", "IS_ALUMODE_INVERTED": "4'b1111",
        "IS_INMODE_INVERTED": "5'b11111", "IS_OPMODE_INVERTED": "7'b1111111",
        "@TIE": {p: "1" for p in ("ALUMODE", "INMODE", "OPMODE", "CARRYIN")},
    },
    # When a register's bypass is written: the bit the database folds into
    # `AREG_0` and `BREG_0`, which a block with one register sets too.
    "in0_zero": {"@TIE": {"INMODE": "partial:00000:00001"}},
    "in4_zero": {"@TIE": {"INMODE": "partial:00000:10000"}},
    "in0_one": {"@TIE": {"INMODE": "partial:00001:00001"}},
    "in4_one": {"@TIE": {"INMODE": "partial:10000:10000"}},
    "in_a2b2": {"AREG": 2, "ACASCREG": 2, "BREG": 2, "BCASCREG": 2, "@TIE": {"INMODE": "0"}},
    "in_a0b0": {"AREG": 0, "ACASCREG": 0, "BREG": 0, "BCASCREG": 0, "@TIE": {"INMODE": "0"}},
    "ce_zero": {"@TIE": {"CEA1": "0", "CEB1": "0"}},
    "selpat_c_odd": {"@LOC": "DSP48_X2Y51", "SEL_PATTERN": '"C"'},
    "in_zero_odd": {"@LOC": "DSP48_X2Y51", "@TIE": {"INMODE": "0"}},
    "comb_open": {"AREG": 0, "ACASCREG": 0, "BREG": 0, "BCASCREG": 0, "CREG": 0, "MREG": 0, "PREG": 0, "OPMODEREG": 0, "ALUMODEREG": 0, "INMODEREG": 0, "CARRYINREG": 0, "CARRYINSELREG": 0, "@NOCLK": 1, "@OPEN": ("C", "D", "CARRYINSEL", "CARRYIN", "CEA1", "CEA2", "CEAD", "CEALUMODE", "CEB1", "CEB2", "CEC", "CECARRYIN", "CECTRL", "CED", "CEINMODE", "CEM", "CEP", "RSTA", "RSTALLCARRYIN", "RSTALUMODE", "RSTB", "RSTC", "RSTCTRL", "RSTD", "RSTINMODE", "RSTM", "RSTP"), "@TIE": {"OPMODE": "partial5", "ALUMODE": "0", "INMODE": "0"}},
    # Left unconnected: what the block reads from a pin nobody names.
    "unconnected": {"@OPEN": ("C", "D", "ALUMODE", "CARRYINSEL", "INMODE", "OPMODE", "CARRYIN", "CEA1", "CEA2", "CEAD", "CEALUMODE", "CEB1", "CEB2", "CEC", "CECARRYIN", "CECTRL", "CED", "CEINMODE", "CEM", "CEP", "RSTA", "RSTALLCARRYIN", "RSTALUMODE", "RSTB", "RSTC", "RSTCTRL", "RSTD", "RSTINMODE", "RSTM", "RSTP")},
}

PORTS = (
    "A:30 B:18 C:48 D:25 ALUMODE:4 CARRYINSEL:3 INMODE:5 OPMODE:7 CARRYIN:1 CEA1:1 CEA2:1 CEAD:1 "
    "CEALUMODE:1 CEB1:1 CEB2:1 CEC:1 CECARRYIN:1 CECTRL:1 CED:1 CEINMODE:1 CEM:1 CEP:1 RSTA:1 "
    "RSTALLCARRYIN:1 RSTALUMODE:1 RSTB:1 RSTC:1 RSTCTRL:1 RSTD:1 RSTINMODE:1 RSTM:1 RSTP:1"
)


def design(name, over):
    params = dict(BASE)
    params.update({k: v for k, v in over.items() if not k.startswith("@")})
    ties = over.get("@TIE", {})
    opened = over.get("@OPEN", ())
    loc = over.get("@LOC", "DSP48_X2Y50")
    conns = []
    at = 0
    for port in PORTS.split():
        key, width = port.split(":")
        width = int(width)
        if key in opened:
            continue
        if key in ties and ties[key] == "partial5":
            conns.append(f".{key}({width}'b0000101)")
            continue
        if key in ties and ties[key].startswith("partial:"):
            # Only the bits of the mask are tied; the rest stay signals.
            _, value, mask = ties[key].split(":")
            value, mask = int(value, 2), int(mask, 2)
            parts = []
            for bit in reversed(range(width)):
                if mask >> bit & 1:
                    parts.append(f"1'b{value >> bit & 1}")
                else:
                    parts.append(f"sr[{at}]")
                    at += 1
            conns.append(f".{key}({{{', '.join(parts)}}})")
            continue
        if key in ties:
            value = (1 << width) - 1 if ties[key] == "1" else 0
            conns.append(f".{key}({width}'h{value:x})")
            continue
        conns.append(f".{key}(sr[{at + width - 1}:{at}])" if width > 1 else f".{key}(sr[{at}])")
        at += width
    if over.get("A_INPUT"):
        conns.append(".ACIN(30'd0)")
    if over.get("B_INPUT"):
        conns.append(".BCIN(18'd0)")
    at = max(at, 2)
    clock = "" if over.get("@NOCLK") else ".CLK(clk), "
    text = ", ".join(f".{k}({v})" for k, v in params.items())
    return (
        f"""module {name} (input wire clk, input wire si, output reg so);
  reg [{at - 1}:0] sr;
  wire [47:0] p;
  reg [47:0] q;
  always @(posedge clk) begin sr <= {{sr[{at - 2}:0], si}}; q <= p; so <= ^q ^ sr[0]; end
  DSP48E1 #({text}) dut ({clock}.P(p), {', '.join(conns)});
endmodule
""",
        "create_clock -period 10 -name clk [get_ports clk]\n"
        "set_property PACKAGE_PIN N11 [get_ports clk]\n"
        "set_property PACKAGE_PIN L4 [get_ports si]\n"
        "set_property PACKAGE_PIN L5 [get_ports so]\n"
        "set_property IOSTANDARD LVCMOS33 [get_ports *]\n"
        "set_property SEVERITY Warning [get_drc_checks {REQP-*}]\n"
        f"set_property LOC {loc} [get_cells dut]\n",
    )


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for name, over in SWEEP.items():
        if sys.argv[2:] and name not in sys.argv[2:]:
            continue
        verilog, xdc = design(name, over)
        with open(os.path.join(out, f"{name}.v"), "w") as f:
            f.write(verilog)
        with open(os.path.join(out, f"{name}.xdc"), "w") as f:
            f.write(xdc)


if __name__ == "__main__":
    main()
