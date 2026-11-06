#!/usr/bin/env python3
"""Writes designs asking where Vivado sets a global track's rebuffer enables and
a clock management tile's `BUFHCLK_USED` on the XC7A35T: flip-flops placed in
chosen clock rows and sides, from a `BUFG` or a `BUFHCE` on a chosen site.
`clock_rules.py` holds Vivado's bitstreams of them to the two rules.

    tools/vivado-parity/gen_clock_rules.py <out>

then `run_all.sh <out> xc7a35tftg256-1 <runs>`. A buffer in the top half
(`BUFGCTRL_X0Y16` and up) takes its clock from `D13`, bank 15's clock-capable
pin in that half; the bottom half's from `N11`. Slices `X10` are left of the
clock column and `X50` right; rows 20, 70 and 120 are the three clock rows.
"""
import os, sys

L0, R0, L1, R1, L2, R2 = ("SLICE_X10Y20", "SLICE_X50Y20", "SLICE_X10Y70", "SLICE_X50Y70",
                          "SLICE_X10Y120", "SLICE_X50Y120")
# (name, the BUFG's site, [(a flip-flop's slice, the BUFHCE it is clocked through or None)])
DESIGNS = [
    ("r_row2_left", "BUFGCTRL_X0Y0", [(L2, None)]),
    ("r_row2_right", "BUFGCTRL_X0Y0", [(R2, None)]),
    ("r_rows02", "BUFGCTRL_X0Y0", [(L0, None), (R2, None)]),
    ("r_all", "BUFGCTRL_X0Y3", [(L0, None), (R0, None), (L1, None), (R1, None), (L2, None), (R2, None)]),
    ("r_top_row0", "BUFGCTRL_X0Y20", [(L0, None)]),
    ("r_top_row2", "BUFGCTRL_X0Y20", [(R2, None)]),
    ("r_top_row1", "BUFGCTRL_X0Y31", [(L1, None)]),
    ("r_bufh_x1_row2", "BUFGCTRL_X0Y0", [(R2, "BUFHCE_X1Y30")]),
    ("r_bufh_x0_row2", "BUFGCTRL_X0Y0", [(L2, "BUFHCE_X0Y29")]),
    ("r_bufh_x1_row0", "BUFGCTRL_X0Y0", [(R0, "BUFHCE_X1Y5")]),
    ("r_bufh_x1_row1", "BUFGCTRL_X0Y17", [(R1, "BUFHCE_X1Y20")]),
]


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for name, bufg, loads in DESIGNS:
        top = int(bufg.split("Y")[-1]) >= 16
        v = [f"module {name} (input wire clk, input wire si, output wire so);", "  wire g;",
             "  BUFG u_g (.I(clk), .O(g));", f"  wire [{len(loads)}:0] c;", "  assign c[0] = si;"]
        xdc = ["create_clock -period 10 -name clk [get_ports clk]",
               f"set_property PACKAGE_PIN {'D13' if top else 'N11'} [get_ports clk]",
               "set_property PACKAGE_PIN L4 [get_ports si]", "set_property PACKAGE_PIN L5 [get_ports so]",
               "set_property IOSTANDARD LVCMOS33 [get_ports {clk si so}]",
               f"set_property LOC {bufg} [get_cells u_g]"]
        for i, (site, bufh) in enumerate(loads):
            clock = "g"
            if bufh:
                v += [f"  wire h{i};", f"  BUFH u_h{i} (.I(g), .O(h{i}));"]
                xdc.append(f"set_property LOC {bufh} [get_cells u_h{i}]")
                clock = f"h{i}"
            v.append(f"  FDRE u_f{i} (.C({clock}), .CE(1'b1), .R(1'b0), .D(c[{i}]), .Q(c[{i + 1}]));")
            xdc.append(f"set_property LOC {site} [get_cells u_f{i}]")
        v += [f"  assign so = c[{len(loads)}];", "endmodule"]
        open(os.path.join(out, f"{name}.v"), "w").write("\n".join(v) + "\n")
        open(os.path.join(out, f"{name}.xdc"), "w").write("\n".join(xdc) + "\n")


main()
