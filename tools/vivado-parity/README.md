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
- `gen_dsp_sweep.py` writes one `DSP48E1` per parameter setting and
  `dsp_sweep_report.py` writes what each did to its tile, as
  `testdata/fpga/vivado/dsp/sweep.txt`.
- `gen_dsp_inference.py` writes one multiply per question of what Vivado puts on
  a block, `dsp_facts.tcl` runs one through it and writes down what became of
  each block, and `dsp_inference_report.py` writes
  `testdata/fpga/vivado/dsp/inference.txt`.
- `dsp_answers.py` keeps what Vivado wrote for each of those blocks, feature by
  feature, with its source, under `answers/` and `designs/` there.
- `gen_dsp_vectors.py` asks `xsim` what the vendor's `DSP48E1` does, a setting at a
  time, and writes `vectors/` there.
- `xsim_routed.py` runs a netlist Reticle routed under `xsim` and the vendor's
  models of every cell, against its source. Nothing of that is committed.
- `gen_slice_cells.py` writes small designs around one latch or shift register
  into `testdata/fpga/vivado/slice_cells/`, and `collect_slice_cells.py` keeps
  what Vivado's bitstream for each decodes to (`reticle fpga --decode`), in
  `vivado.features` there.
- `gen_iologic.py` writes `testdata/fpga/vivado/iologic/`, an IO register or a serialiser
  per pad, and `tests/fpga_xray_ioreg.rs` decodes `run_all.sh`'s bitstreams of it into
  `vivado.txt`.
- `gen_idelay.py` writes `testdata/fpga/vivado/idelay/`, an input delay per pad and the
  designs about its controller and `DCIRESET`, and `tests/fpga_xray_idelay.rs` decodes
  `run_all.sh`'s bitstreams of it into `vivado.txt`.
- `gen_io.py` writes `testdata/fpga/vivado/io/`, and `io_pairs.sh` with `io_pairs.tcl`,
  `io_pairs.v` and `io_pairs.py` asks Vivado's design rule check about every pair of
  (standard, direction) in one bank, for `pairs.txt` there. `tests/fpga_xray_pads.rs`
  decodes `run_all.sh`'s bitstreams into `vivado.txt`.
- `gen_clocks.py` writes `testdata/fpga/vivado/clocks/`, `clocks.sh` with
  `clocks_probe.tcl` asks Vivado about each of those designs, and `clocks_fixture.py`
  keeps its answer, read back by `--decode`, in `vivado.txt` there. `cmt_sweep.sh`,
  `cmt_sweep.tcl` and `cmt_sweep.py` change one parameter at a time of a clock manager
  `clocks.sh` routed, and keep the registers of each in `registers.txt` there.
- `gen_lutram.py` writes `testdata/fpga/vivado/lutram/`, and `lutram.sh` with
  `lutram_probe.tcl` asks Vivado what it makes of those designs: each memory
  cell's leaves, bels and site pins, a bitstream, and its routed netlist on its
  own simulation models, which `gen_lutram_vectors.py` runs under `xsim` into
  `xsim.txt` there. `gen_lutram.py --sweep` and `lutram_sweep.tcl` ask its
  synthesis which memories go to block RAM, into `sweep.txt`;
  `tests/fpga_xray_lutram_vivado.rs` decodes the bitstreams into `vivado.txt`
  and holds Reticle to all of it.
- `gen_bram.py` writes `testdata/fpga/vivado/bram/`, block RAMs by name in each
  mode and memories to infer as one, and `FIXTURE=bram lutram.sh` asks Vivado the
  same of them; `gen_lutram_vectors.py` simulates them too, and
  `tests/fpga_xray_bram_vivado.rs` decodes the bitstreams into `vivado.txt` there.
- `gen_fifo.py` writes `testdata/fpga/vivado/fifo/`, a `FIFO18E1` or `FIFO36E1` by
  name per setting, and `FIXTURE=fifo lutram.sh` asks Vivado the same of them
  (`lutram_probe.tcl` also lists every pin of a FIFO's site and its net);
  `gen_lutram_vectors.py` simulates them, and `tests/fpga_xray_fifo_vivado.rs`
  decodes the bitstreams into `vivado.txt` there. `gen_fifo.py --ranges` writes
  designs whose flag offsets Vivado refuses, and `fifo_ranges.py` keeps the range
  each refusal names, as `ranges.txt`; `gen_fifo.py --values` writes designs of
  random initial values, and `fifo_values.py` solves which bit of `INIT` each of the
  tile's register bits is, as `values.txt`.
- `gen_config.py` writes `testdata/fpga/vivado/config/`, one configuration primitive
  a design, `config.sh` with `config_probe.tcl` asks Vivado about each, and
  `config_fixture.py` keeps its answer, read back by `--decode`, with the stream's
  packets by `bit_packets.py`, in `vivado.txt` there.
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
