#!/usr/bin/env python3
"""Asks Vivado's simulator what a DSP48E1 does, a setting at a time.

For each setting below, this drives the vendor's model with fixed pseudo-random
data, enables and resets under `xsim` and writes
testdata/fpga/vivado/dsp/vectors/<name>.v: a testbench holding the inputs and the
outputs that came back, which instantiates the block by name and prints
`MISMATCH` if an output differs. Nothing of the vendor's model is written out,
only what it answered.

    tools/vivado-parity/gen_dsp_vectors.py ~/Xilinx/2026.1/Vivado <workdir>
"""
import os
import random
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "testdata/fpga/vivado/dsp/vectors")
STEPS = 32
SETTLE = 200  # ns: the vendor's global reset lasts 100 ns.

MULTIPLY = {"OPMODE": "7'b0000101", "ALUMODE": "4'b0000", "CARRYINSEL": "3'b000", "INMODE": "5'b00000"}
ALLREG0 = dict(ACASCREG=0, ADREG=0, ALUMODEREG=0, AREG=0, BCASCREG=0, BREG=0, CARRYINREG=0, CARRYINSELREG=0, CREG=0, DREG=0, INMODEREG=0, MREG=0, OPMODEREG=0, PREG=0)


def config(regs=None, ties=None, params=None, carry=False, detect=False):
    """A setting: registers over all-off, the mode pins, other parameters."""
    p = dict(ALLREG0)
    p.update(regs or {})
    p.update(params or {})
    t = dict(MULTIPLY)
    t.update(ties or {})
    return dict(params=p, ties=t, carry=carry, detect=detect)


def areg(n):
    return dict(AREG=n, ACASCREG=n)


def breg(n):
    return dict(BREG=n, BCASCREG=n)


SETTINGS = {
    # The multiply, with each register.
    "mul_comb": config(),
    "mul_a1": config(areg(1)),
    "mul_a2": config(areg(2)),
    "mul_a2c1": config(dict(AREG=2, ACASCREG=1)),
    "mul_b1": config(breg(1)),
    "mul_b2": config(breg(2)),
    "mul_b2c1": config(dict(BREG=2, BCASCREG=1)),
    "mul_m": config(dict(MREG=1)),
    "mul_p": config(dict(PREG=1)),
    "mul_mp": config(dict(MREG=1, PREG=1)),
    "mul_a1b1m": config({**areg(1), **breg(1), "MREG": 1}),
    "mul_a1b1p": config({**areg(1), **breg(1), "PREG": 1}),
    "mul_a2b2mp": config({**areg(2), **breg(2), "MREG": 1, "PREG": 1}),
    "mul_a2b1": config({**areg(2), **breg(1)}),
    # What is added to it.
    "add_c": config(ties={"OPMODE": "7'b0110101"}),
    "add_c_reg": config({"CREG": 1, "PREG": 1}, ties={"OPMODE": "7'b0110101"}),
    "acc_p": config({"PREG": 1}, ties={"OPMODE": "7'b0100101"}),
    "add_pcin": config(ties={"OPMODE": "7'b0010101"}),
    "add_pcin17": config(ties={"OPMODE": "7'b1010101"}),
    "add_pcin17_reg": config({"MREG": 1, "PREG": 1}, ties={"OPMODE": "7'b1010101"}),
    "acc_p17": config({"PREG": 1}, ties={"OPMODE": "7'b1100101"}),
    "acc_p100": config({"PREG": 1}, ties={"OPMODE": "7'b1000101"}),
    # The ALU.
    "sub_c": config(ties={"OPMODE": "7'b0110101", "ALUMODE": "4'b0011"}),
    "alu1_c": config(ties={"OPMODE": "7'b0110101", "ALUMODE": "4'b0001"}),
    "alu2_c": config(ties={"OPMODE": "7'b0110101", "ALUMODE": "4'b0010"}),
    # No multiplier: A:B and C, with the carries.
    "ab_c": config(ties={"OPMODE": "7'b0110011"}, params={"USE_MULT": '"NONE"'}, carry=True),
    # Not its carries: see `dsp48e1_model.v`.
    "ab_ones": config(ties={"OPMODE": "7'b0001011"}, params={"USE_MULT": '"NONE"'}),
    "ab_c_reg": config({**areg(1), **breg(1), "CREG": 1, "PREG": 1}, ties={"OPMODE": "7'b0110011"}, params={"USE_MULT": '"NONE"'}, carry=True),
    "c_c": config(ties={"OPMODE": "7'b0111100"}, params={"USE_MULT": '"NONE"'}, carry=True),
    "x_p": config({"PREG": 1}, ties={"OPMODE": "7'b0110010"}, params={"USE_MULT": '"NONE"'}),
    "ci_pcin": config(ties={"OPMODE": "7'b0110011", "CARRYINSEL": "3'b001"}, params={"USE_MULT": '"NONE"'}, carry=True),
    "ci_npcin": config(ties={"OPMODE": "7'b0110011", "CARRYINSEL": "3'b011"}, params={"USE_MULT": '"NONE"'}, carry=True),
    "ci_np": config({"PREG": 1}, ties={"OPMODE": "7'b0110011", "CARRYINSEL": "3'b101"}, params={"USE_MULT": '"NONE"'}, carry=True),
    "ci_p": config({"PREG": 1}, ties={"OPMODE": "7'b0110011", "CARRYINSEL": "3'b111"}, params={"USE_MULT": '"NONE"'}, carry=True),
    "dyn_mult": config(dict(MREG=1, PREG=1), params={"USE_MULT": '"DYNAMIC"'}),
    # The cascades.
    "cas_a": config(areg(1), params={"A_INPUT": '"CASCADE"'}),
    "cas_b": config(breg(1), params={"B_INPUT": '"CASCADE"'}),
    # INMODE.
    "in_a1": config({**areg(2), "MREG": 1}, ties={"INMODE": "5'b00001"}),
    "in_zero": config(ties={"INMODE": "5'b00010"}),
    "in_b1": config({**breg(2), "MREG": 1}, ties={"INMODE": "5'b10000"}),
    "in_add": config(ties={"INMODE": "5'b00100"}, params={"USE_DPORT": '"TRUE"'}),
    "in_sub": config(ties={"INMODE": "5'b01100"}, params={"USE_DPORT": '"TRUE"'}),
    "in_add_reg": config({"DREG": 1, "ADREG": 1, **areg(1), **breg(1), "MREG": 1, "PREG": 1}, ties={"INMODE": "5'b00100"}, params={"USE_DPORT": '"TRUE"'}),
    # The control registers.
    "ctl_regs": config(dict(OPMODEREG=1, ALUMODEREG=1, INMODEREG=1, CARRYINSELREG=1, CARRYINREG=1), ties={"OPMODE": "7'b0110101"}),
    "ci_reg": config(dict(CARRYINREG=1), ties={"OPMODE": "7'b0110011"}, params={"USE_MULT": '"NONE"'}),
    # The inverters.
    "inv_clk": config({**areg(1), **breg(1), "MREG": 1, "PREG": 1}, params={"IS_CLK_INVERTED": "1'b1"}),
    "inv_op": config(ties={"OPMODE": "7'b1001010"}, params={"IS_OPMODE_INVERTED": "7'b1111111"}),
    "inv_alu": config(ties={"ALUMODE": "4'b1111"}, params={"IS_ALUMODE_INVERTED": "4'b1111"}),
    "inv_in": config(ties={"INMODE": "5'b11111"}, params={"IS_INMODE_INVERTED": "5'b11111"}),
    "inv_ci": config(ties={"OPMODE": "7'b0110011"}, params={"USE_MULT": '"NONE"', "IS_CARRYIN_INVERTED": "1'b1"}),
    # The pattern detector.
    "pat_mask": config(ties={"OPMODE": "7'b0110011"}, params={"USE_MULT": '"NONE"', "USE_PATTERN_DETECT": '"PATDET"', "MASK": "48'hfffffffffff0", "PATTERN": "48'h000000000005"}, detect=True),
    "pat_reg": config({"PREG": 1}, ties={"OPMODE": "7'b0110011"}, params={"USE_MULT": '"NONE"', "USE_PATTERN_DETECT": '"PATDET"', "MASK": "48'hfffffffffff0", "PATTERN": "48'h000000000005"}, detect=True),
}

INPUTS = (("A", 30), ("ACIN", 30), ("B", 18), ("BCIN", 18), ("C", 48), ("D", 25), ("PCIN", 48), ("CARRYIN", 1),
          ("CARRYCASCIN", 1), ("MULTSIGNIN", 1))
ENABLES = ("CEA1", "CEA2", "CEAD", "CEALUMODE", "CEB1", "CEB2", "CEC", "CECARRYIN", "CECTRL", "CED", "CEINMODE", "CEM", "CEP")
RESETS = ("RSTA", "RSTALLCARRYIN", "RSTALUMODE", "RSTB", "RSTC", "RSTCTRL", "RSTD", "RSTINMODE", "RSTM", "RSTP")
OUTPUTS = (("P", 48), ("PCOUT", 48), ("ACOUT", 30), ("BCOUT", 18), ("CARRYOUT", 4), ("CARRYCASCOUT", 1), ("PATTERNDETECT", 1), ("PATTERNBDETECT", 1))


def head(name, setting):
    """The testbench down to the block; it returns input and output widths."""
    iw = sum(w for _, w in INPUTS) + len(ENABLES) + len(RESETS)
    ow = sum(w for _, w in OUTPUTS)
    lines = [f"// DSP48E1 {name}: inputs and Vivado's simulator's answers.",
             "// Generated by tools/vivado-parity/gen_dsp_vectors.py.", "`timescale 1ns / 1ps", "module tb;",
             f"  reg [{iw - 1}:0] i = 0;", "  reg clk = 0;", f"  wire [{ow - 1}:0] o;"]
    conns = [".CLK(clk)"]
    at = 0
    for n, w in INPUTS:
        conns.append(f".{n}(i[{at + w - 1}:{at}])" if w > 1 else f".{n}(i[{at}])")
        at += w
    for n in ENABLES + RESETS:
        conns.append(f".{n}(i[{at}])")
        at += 1
    at = 0
    for n, w in OUTPUTS:
        conns.append(f".{n}(o[{at + w - 1}:{at}])" if w > 1 else f".{n}(o[{at}])")
        at += w
    for pin, value in setting["ties"].items():
        conns.append(f".{pin}({value})")
    params = ", ".join(f".{k}({v})" for k, v in setting["params"].items())
    lines.append(f"  DSP48E1 #({params}) dut ({', '.join(conns)});")
    return lines, iw, ow


def stimulus(rng, iw, steps):
    """Data at random, enables mostly on, resets mostly off."""
    data_bits = sum(w for _, w in INPUTS)
    out = []
    for _ in range(steps):
        value = rng.getrandbits(data_bits)
        at = data_bits
        for _ in ENABLES:
            if rng.random() < 0.85:
                value |= 1 << at
            at += 1
        for _ in RESETS:
            if rng.random() < 0.06:
                value |= 1 << at
            at += 1
        out.append(value)
    return out


def body(iw, ow, stim, answers=None):
    if answers is None:
        out = ["  initial begin", f"    #{SETTLE};"]
        for n, s in enumerate(stim):
            out += [f"    i = {iw}'h{s:x}; #4; clk = 1; #4; clk = 0; #2;", f'    $display("R {n} %h", o);']
        return out + ["    $finish;", "  end", "endmodule"]
    out = ["  integer bad = 0;", "  integer n = 0;",
           f"  task t(input [{iw - 1}:0] v, input [{ow - 1}:0] want, input [{ow - 1}:0] mask);",
           "    begin", "      i = v; #4; clk = 1; #4; clk = 0; #2;",
           "      if ((o & mask) !== (want & mask)) begin",
           '        bad = bad + 1; $display("MISMATCH %0d %h", n, o);', "      end", "      n = n + 1;", "    end", "  endtask",
           "  initial begin", f"    #{SETTLE};"]
    for n, s in enumerate(stim):
        want, mask = answers[n]
        out.append(f"    t('h{s:x}, 'h{want:x}, 'h{mask:x});")
    out += ['    if (bad == 0) $display("PASS"); else $display("FAIL");', "    $finish;", "  end", "endmodule"]
    return out


def asked_mask(setting):
    """The outputs a setting asks about: the carries only without a multiplier,
    whose partial products make them meaningless, and the pattern detector only
    when it is on."""
    mask = 0
    at = 0
    for name, width in OUTPUTS:
        ask = {"CARRYOUT": setting["carry"], "CARRYCASCOUT": setting["carry"],
               "PATTERNDETECT": setting["detect"], "PATTERNBDETECT": setting["detect"]}.get(name, True)
        if ask:
            mask |= ((1 << width) - 1) << at
        at += width
    return mask


def run(job):
    vivado, work, name, setting, seed = job
    rng = random.Random(seed)
    lines, iw, ow = head(name, setting)
    stim = stimulus(rng, iw, STEPS)
    d = os.path.join(work, name)
    os.makedirs(d, exist_ok=True)
    with open(f"{d}/tb.v", "w") as f:
        f.write("\n".join(lines + body(iw, ow, stim)) + "\n")
    glbl = f"{vivado}/data/verilog/src/glbl.v"
    script = (f". {vivado}/settings64.sh >/dev/null 2>&1; cd {d}; "
              f"nice -n 19 ionice -c3 xvlog -sv tb.v {glbl} >x.log 2>&1 && "
              f"nice -n 19 ionice -c3 xelab -L unisims_ver -debug off tb glbl -s s >>x.log 2>&1 && "
              f"nice -n 19 ionice -c3 xsim s -R >out.log 2>&1")
    subprocess.run(["sh", "-c", script], check=False)
    try:
        text = open(f"{d}/out.log").read().splitlines()
    except OSError:
        return name, None
    answers = {}
    asked = asked_mask(setting)
    for line in text:
        m = re.match(r"R (\d+) (\S+)$", line)
        if m:
            digits = m.group(2).lower().replace("z", "x").rjust((ow + 3) // 4, "0")
            mask = int("".join("0" if c == "x" else "f" for c in digits), 16) & asked
            want = int(digits.replace("x", "0"), 16) & mask
            answers[int(m.group(1))] = (want, mask)
    if len(answers) != STEPS:
        return name, None
    with open(f"{OUT}/{name}.v", "w") as f:
        f.write("\n".join(lines + body(iw, ow, stim, answers)) + "\n")
    return name, True


def main():
    vivado, work = sys.argv[1:3]
    os.makedirs(OUT, exist_ok=True)
    rng = random.Random(0xd5b)
    jobs = [(vivado, work, name, setting, rng.getrandbits(32)) for name, setting in SETTINGS.items()]
    with ThreadPoolExecutor(int(os.environ.get("JOBS", "16"))) as pool:
        results = list(pool.map(run, jobs))
    missing = [n for n, ok in results if not ok]
    print(len(results), "written;", len(missing), "the simulator would not run:", " ".join(missing))


main()
