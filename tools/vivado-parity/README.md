# Vivado as the oracle

The scripts that ask Vivado 2026.1 what it does with designs written here, so
that Reticle can be held to the answer. None of it is needed to run the tests:
what they compare with is committed, under `testdata/fpga/vivado/`. Only
answers are committed, about designs written here; nothing of the vendor's
models or IP.

- `gen_cells.py` writes `src/fpga/devices/xc7_cells.v`.
- `gen_cell_vectors.py` asks `xsim` what each of those cells does, and writes
  `testdata/fpga/vivado/cells/`.
- `gen_primitives.py` writes one design per 7-series primitive into
  `testdata/fpga/vivado/primitives/`.
- `gen_xdc.py` and `xdc_answers.tcl` ask what Vivado makes of each constraints
  file in `testdata/fpga/vivado/xdc/`, and keep the answer beside it.
- `gen_muxf.py` writes `testdata/fpga/vivado/muxf/`, and `muxf.sh` with
  `muxf_probe.tcl` asks Vivado what it makes of those designs, in its own report
  and in a bitstream; `tests/fpga_xray_muxf.rs` decodes the bitstreams into
  `vivado.txt` there and holds Reticle to it.
- `run_all.sh` and `run_vivado.tcl` run a directory of designs through Vivado;
  `run_reticle.sh` does the same with Reticle. Both run at the lowest priority.
- `run_placed.sh` and `run_placed.tcl` give Vivado Reticle's own implementation
  (`reticle fpga --vivado-placement`) to place, route and time, or to time as it
  is. `decode_placed.sh` and `compare_bits.py` compare the bitstreams, feature by
  feature, `frame_ecc.py` checks what those leave over, and `placed_table.py` and
  `bits_table.py` write `testdata/fpga/vivado/placed/`.
- `bitstream_base.tcl` routes `testdata/fpga/vivado/bitstream/options_probe.v` once;
  `bitstream_options.tcl` writes a `.bit` from that checkpoint per line of
  `xc7a100t.runs.txt` or `xc7a35t.runs.txt` there; `bit_packets.py` prints a stream's
  packets, and `bitstream_fixtures.py` turns all of it into that directory's answers.
- `gen_ecc_design.py` and `bitstream_ecc.tcl` make a design with many bits set, for the
  frames of `ecc_frames.txt`. `bitstream_ll.tcl` makes `ll_probe`'s logic location file.

`docs/fpga-vivado-parity.md` has what the answers say.
