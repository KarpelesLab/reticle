//! When a global clock reaches a slice of the `xc7a100t`, against what Vivado
//! said: `testdata/fpga/vivado/clock/arrival.txt`, written by
//! `tools/vivado-parity/clock_arrival.tcl` and `clock_arrival.py fixture`, the
//! clock net's delay to 2,100 flip-flops over the die from global buffers at
//! three sites. Vivado is not needed: the fixture carries its answers.

#![cfg(feature = "fpga")]

use std::path::PathBuf;

/// **Every flip-flop Vivado timed is within 6 ps of the table**, once the
/// table's zero, the earliest arrival, is set against Vivado's insertion delay
/// (the median difference). The table is the fixture's medians by clock region
/// row and column, so this holds that the growth along a column, the region
/// rows and the columns between those measured are what Vivado's are: a row
/// read from the wrong region row is off by 60 to 230 ps, a column by up to 80,
/// the growth left out by 19 at a region's edge. Would not catch the table
/// being applied to a part it was not measured on, which
/// `the_clock_reaches_a_site_when_the_table_says` (`xray::timing`) holds.
#[test]
fn every_flip_flop_vivado_timed_is_within_six_picoseconds_of_the_table() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/vivado/clock/arrival.txt");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!(
            "skipped: `{}` is not in the published crate",
            path.display()
        );
        return;
    };
    let mut differences = Vec::new();
    let mut buffers = std::collections::BTreeSet::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.as_slice() {
            ["hclk", _] => {}
            ["arrival", buffer, site, x, y, ps] => {
                buffers.insert((*buffer).to_owned());
                let (x, y): (u32, u32) = (x.parse().unwrap(), y.parse().unwrap());
                let ps: f64 = ps.parse().unwrap();
                differences.push((
                    ps - reticle::fpga::xray::clock_arrival_ps(x, y),
                    (*site).to_owned(),
                ));
            }
            other => panic!("a line of arrival.txt this does not know: {other:?}"),
        }
    }
    assert_eq!(buffers.len(), 3, "three global buffers: {buffers:?}");
    assert_eq!(differences.len(), 2100);
    let mut sorted: Vec<f64> = differences.iter().map(|(d, _)| *d).collect();
    sorted.sort_by(f64::total_cmp);
    let insertion = sorted[sorted.len() / 2];
    for (difference, site) in &differences {
        assert!(
            (difference - insertion).abs() <= 6.0,
            "{site}: {:.1} ps from the table",
            difference - insertion
        );
    }
}
