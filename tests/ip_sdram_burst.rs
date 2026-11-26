//! `ip/memory/sdram_burst`, simulated against a model of the Winbond
//! W9825G6KH that checks every command the controller sends.
//!
//! # The model
//!
//! [`Sdram`] is written from the W9825G6KH datasheet (revision A04, 20
//! March 2017) and from nothing else. It is a memory — every word written
//! is stored and every read returns what is stored — and it is a checker:
//! each command is held to the rule of §7, §8 and §9.5 it could break, and
//! any break is an error that fails the test with the cycle and the rule.
//! The rules, each with the section it comes from:
//!
//! | rule | source |
//! |---|---|
//! | 200 µs of NOP with DQM and CKE high before the first command, and that command PRECHARGE ALL | §7.1 |
//! | MODE REGISTER SET with every bank idle, tRSC (2 cycles) before the next command, and eight AUTO REFRESH before the first ACTIVE | §7.1, §7.2, §9.5 |
//! | the mode is full page, sequential, burst write, and the CAS latency the bench expects | §10.4 |
//! | ACTIVE only to an idle bank, tRC after its last ACTIVE, tRP after its PRECHARGE, tRRD (2 cycles) after any ACTIVE, tRC after AUTO REFRESH | §7.3, §9.5 |
//! | READ and WRITE only to an open bank, tRCD after its ACTIVE, never with A10 (auto-precharge is illegal in full page) | §7.4, §7.14 |
//! | PRECHARGE tRAS after ACTIVE and tWR (2 cycles) after the last data written | §7.15, §9.5, chart §11.20 |
//! | AUTO REFRESH with every bank idle and tRP after the precharge; never more than 64 ms / 8192 between two | §7.16, §9.5 |
//! | no row open longer than tRAS(max), 100 µs | §9.5 |
//! | a burst ends at READ, WRITE, BURST STOP or a PRECHARGE of its bank; a read's data comes CAS latency later, and stops CAS latency after what ended it | §7.7, §7.9, §7.11, charts §11.19, §11.20 |
//! | DQM masks write data at once and read data two cycles later | §5, §10.3 |
//! | no command with CKE low | §8 |
//!
//! Three rules are the controller's own and not the datasheet's, and are
//! marked as such in the errors: no burst longer than the sixteen words
//! the port allows or wrapping round its row; a write data cycle has the
//! bus driven; and **bus contention**. For the last the model draws the
//! bus in half cycles: the part drives a read word from the clock edge it
//! is launched on until tHZ (5 ns) after the next, and the FPGA drives a
//! cycle whose output enable is high from half a cycle before that
//! cycle's edge (the inverted forwarded clock) to a whole cycle after the
//! end of it (its three-state control is a fabric register, slower than
//! the data's). Any overlap is an error.
//!
//! # What these would and would not catch
//!
//! `the_model_catches_*` feeds the model hand-made command sequences that
//! break each rule once and asserts that it says so, and one that breaks
//! none, so a model that let everything through would fail. The bench
//! tests then run the controller against it. They would catch a timing
//! the controller breaks under the traffic they generate, a word read
//! back wrong or given to the wrong port or out of order, and a port
//! starved. They would not catch a break the traffic never provokes, or a
//! fault in the model's reading of the datasheet that the controller
//! shares — the README lists which parts of the datasheet the model
//! quotes and has not measured.

#![cfg(all(feature = "ip", feature = "sim", feature = "verilog", feature = "fpga"))]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::path::Path;

use reticle::diag::Diagnostics;
use reticle::ip::IpManifest;
use reticle::ir::Design;
use reticle::logic::Logic;
use reticle::sim::{NetHandle, SimOptions, Simulator};
use reticle::source::SourceMap;
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

// ---------------------------------------------------------------------------
// Building the design
// ---------------------------------------------------------------------------

fn package_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ip/memory/sdram_burst")
}

/// The package's sources, as its manifest lists them.
fn sources() -> Vec<(String, String)> {
    let dir = package_dir();
    let text = fs::read_to_string(dir.join("reticle.ip"))
        .expect("the manifest")
        .replace("\r\n", "\n");
    let mut map = SourceMap::new();
    let file = map.add("reticle.ip", &text).expect("fits");
    let mut diags = Diagnostics::new();
    let ip = IpManifest::parse(&text, file, &mut diags).expect("the manifest parses");
    ip.sources
        .iter()
        .map(|s| {
            let path = dir.join(&s.path);
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                .replace("\r\n", "\n");
            (path.display().to_string(), text)
        })
        .collect()
}

/// Elaborates `top` with parameter overrides, plus any extra sources.
fn design(top: &str, params: &[(&str, String)], extra: &[(String, String)]) -> Design {
    let mut all = sources();
    all.extend(extra.iter().cloned());
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut files = Vec::new();
    for (path, text) in &all {
        let id = map.add(path.clone(), text).expect("source fits");
        files.push(parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        ));
    }
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let mut options = ElabOptions::new(Dialect::Verilog2005).with_top(top);
    for (name, value) in params {
        options = options.with_param(*name, value.as_str());
    }
    let refs: Vec<_> = files.iter().collect();
    let design = elaborate(&refs, &options, &mut diags);
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    design.expect("a design")
}

fn bit(v: bool) -> Logic {
    Logic::from_bool(v)
}

/// A value of any width from a little-endian list of 64-bit words.
fn wide(width: u32, words: &[u64]) -> Logic {
    let n = width.div_ceil(64) as usize;
    let mut value = vec![0u64; n];
    for (i, w) in words.iter().enumerate().take(n) {
        value[i] = *w;
    }
    if !width.is_multiple_of(64) {
        value[n - 1] &= (1u64 << (width % 64)) - 1;
    }
    Logic::from_planes(width, false, value, vec![0u64; n])
}

/// Bits `[lo, lo + w)` of a net's value, `w` at most 64.
fn field(v: &Logic, lo: u32, w: u32) -> u64 {
    let mut out = 0u64;
    for i in 0..w {
        match v.bit(lo + i) {
            reticle::logic::Bit::One => out |= 1 << i,
            reticle::logic::Bit::Zero => {}
            other => panic!("bit {} of a net is {other:?}", lo + i),
        }
    }
    out
}

/// A small deterministic generator, so every run is the same run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    fn below32(&mut self, n: u32) -> u32 {
        cast(self.below(u64::from(n)))
    }
}

/// A value narrowed to the type it is known to fit.
fn cast<T: TryFrom<u64>>(v: u64) -> T
where
    T::Error: std::fmt::Debug,
{
    T::try_from(v).expect("the value fits")
}

/// An index narrowed to thirty-two bits.
fn n32(v: usize) -> u32 {
    u32::try_from(v).expect("the index fits")
}

/// What the part holds before anything is written: a hash of the
/// address, so a read of a word nobody wrote is checked too.
fn initial(addr: u32) -> u16 {
    let mut x = u64::from(addr).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 29;
    (x & 0xFFFF) as u16
}

// ---------------------------------------------------------------------------
// The SDRAM model
// ---------------------------------------------------------------------------

/// The datasheet's numbers (§9.5, the -6 column), in picoseconds or
/// cycles as §9.5 gives them.
#[derive(Clone, Copy)]
struct Timing {
    tck_ps: u64,
    cas: u64,
    init_ps: u64,
    rcd_ps: u64,
    rp_ps: u64,
    ras_ps: u64,
    ras_max_ps: u64,
    rc_ps: u64,
    refi_ps: u64,
    rrd: u64,
    wr: u64,
    rsc: u64,
}

impl Timing {
    fn w9825g6kh6(mhz: u64, cas: u64) -> Timing {
        Timing {
            tck_ps: 1_000_000 / mhz,
            cas,
            init_ps: 200_000_000,
            rcd_ps: 15_000,
            rp_ps: 15_000,
            ras_ps: 42_000,
            ras_max_ps: 100_000_000,
            rc_ps: 60_000,
            // 64 ms over 8192 rows.
            refi_ps: 7_812_500,
            rrd: 2,
            wr: 2,
            rsc: 2,
        }
    }
}

/// One cycle of the pins.
#[derive(Clone, Copy, Debug, Default)]
struct Pins {
    cke: bool,
    cs_n: bool,
    ras_n: bool,
    cas_n: bool,
    we_n: bool,
    ba: usize,
    a: u16,
    dqm: u8,
    dq: u16,
    oe: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmd {
    Inhibit,
    Nop,
    Active,
    Read,
    Write,
    Precharge,
    Refresh,
    Mode,
    Stop,
}

impl Pins {
    fn cmd(&self) -> Cmd {
        if self.cs_n {
            return Cmd::Inhibit;
        }
        match (self.ras_n, self.cas_n, self.we_n) {
            (true, true, true) => Cmd::Nop,
            (false, true, true) => Cmd::Active,
            (true, false, true) => Cmd::Read,
            (true, false, false) => Cmd::Write,
            (false, true, false) => Cmd::Precharge,
            (false, false, true) => Cmd::Refresh,
            (false, false, false) => Cmd::Mode,
            (true, true, false) => Cmd::Stop,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Bank {
    row: Option<u16>,
    act_at: Option<u64>,
    pre_at: Option<u64>,
    wr_at: Option<u64>,
}

#[derive(Clone, Copy)]
struct Burst {
    read: bool,
    bank: usize,
    row: u16,
    col: u16,
    start: u64,
}

/// A word on the bus from the part: which cycle, the value, and which
/// bytes DQM turned off.
#[derive(Clone, Copy)]
struct Out {
    value: u16,
    masked: u8,
}

struct Sdram {
    t: Timing,
    banks: [Bank; 4],
    last_act: Option<u64>,
    last_ref: Option<u64>,
    last_mrs: Option<u64>,
    first_cke: Option<u64>,
    started: bool,
    refs: u32,
    mode: Option<u16>,
    burst: Option<Burst>,
    outs: BTreeMap<u64, Out>,
    dqm: BTreeMap<u64, u8>,
    mem: HashMap<u32, u16>,
    errors: Vec<String>,
    /// Data cycles on the bus, read and written, and commands by kind.
    words: u64,
    count: BTreeMap<&'static str, u64>,
    ref_times: Vec<u64>,
}

impl Sdram {
    fn new(t: Timing) -> Sdram {
        Sdram {
            t,
            banks: [Bank::default(); 4],
            last_act: None,
            last_ref: None,
            last_mrs: None,
            first_cke: None,
            started: false,
            refs: 0,
            mode: None,
            burst: None,
            outs: BTreeMap::new(),
            dqm: BTreeMap::new(),
            mem: HashMap::new(),
            errors: Vec::new(),
            words: 0,
            count: BTreeMap::new(),
            ref_times: Vec::new(),
        }
    }

    fn err(&mut self, n: u64, what: String) {
        if self.errors.len() < 50 {
            self.errors.push(format!("cycle {n}: {what}"));
        }
    }

    /// `n - then` cycles are at least `ps` picoseconds.
    fn elapsed(&self, n: u64, then: Option<u64>, ps: u64) -> bool {
        then.is_none_or(|t| (n - t) * self.t.tck_ps >= ps)
    }

    fn word(&self, bank: usize, row: u16, col: u16) -> u32 {
        (u32::from(row) << 11) | (n32(bank) << 9) | u32::from(col)
    }

    fn read_mem(&self, addr: u32) -> u16 {
        *self.mem.get(&addr).unwrap_or(&initial(addr))
    }

    /// What the part drives in cycle `k`, if anything.
    fn output(&self, k: u64) -> Option<Out> {
        self.outs.get(&k).copied()
    }

    fn step(&mut self, n: u64, p: Pins) {
        let cmd = p.cmd();
        self.dqm.insert(n, p.dqm);
        // Keep the histories short.
        if n > 64 {
            let old = n - 64;
            self.dqm.retain(|&k, _| k >= old);
            self.outs.retain(|&k, _| k >= old);
        }

        // The power-up pause (§7.1).
        if p.cke && self.first_cke.is_none() {
            self.first_cke = Some(n);
        }
        if !p.cke && self.first_cke.is_some() {
            self.err(
                n,
                "CKE fell: the controller never suspends the clock (§7.20)".into(),
            );
        }
        if !p.cke && !matches!(cmd, Cmd::Inhibit | Cmd::Nop) {
            self.err(n, format!("{cmd:?} with CKE low (§8)"));
        }
        if !self.started {
            if matches!(cmd, Cmd::Inhibit | Cmd::Nop) {
                if self.first_cke.is_some() && p.dqm != 3 {
                    self.err(n, "DQM low during the power-up pause (§7.1)".into());
                }
                return;
            }
            self.started = true;
            let from = self.first_cke.unwrap_or(n);
            if (n - from) * self.t.tck_ps < self.t.init_ps {
                self.err(
                    n,
                    format!(
                        "first command {} ns after CKE rose, before the {} µs pause (§7.1)",
                        (n - from) * self.t.tck_ps / 1000,
                        self.t.init_ps / 1_000_000
                    ),
                );
            }
            if !(cmd == Cmd::Precharge && p.a & 0x400 != 0) {
                self.err(
                    n,
                    format!("first command {cmd:?}, not PRECHARGE ALL (§7.1)"),
                );
            }
        }
        if let Some(m) = self.last_mrs
            && !matches!(cmd, Cmd::Inhibit | Cmd::Nop)
            && n - m < self.t.rsc
        {
            self.err(
                n,
                format!("{cmd:?} {} cycle(s) after MODE REGISTER SET: tRSC", n - m),
            );
        }
        *self.count.entry(cmd_name(cmd)).or_insert(0) += 1;

        // The burst in progress: this cycle is its data cycle unless the
        // command ends it here.
        let ends = match cmd {
            Cmd::Read | Cmd::Write | Cmd::Stop => true,
            Cmd::Precharge => self
                .burst
                .is_some_and(|b| p.a & 0x400 != 0 || b.bank == p.ba),
            _ => false,
        };
        if let Some(b) = self.burst {
            if ends {
                self.burst = None;
            } else {
                let i = n - b.start;
                self.slot(n, b, i, p);
            }
        }

        match cmd {
            Cmd::Inhibit | Cmd::Nop | Cmd::Stop => {}
            Cmd::Active => self.active(n, p),
            Cmd::Read | Cmd::Write => {
                let bank = p.ba;
                let b = self.banks[bank];
                match b.row {
                    None => self.err(n, format!("{cmd:?} to bank {bank}, which is idle (§7.4)")),
                    Some(row) => {
                        if !self.elapsed(n, b.act_at, self.t.rcd_ps) {
                            self.err(n, format!("{cmd:?} to bank {bank} before tRCD"));
                        }
                        if p.a & 0x400 != 0 {
                            self.err(n, "auto-precharge in full-page mode (§7.14)".into());
                        }
                        let burst = Burst {
                            read: cmd == Cmd::Read,
                            bank,
                            row,
                            col: p.a & 0x1FF,
                            start: n,
                        };
                        self.burst = Some(burst);
                        self.slot(n, burst, 0, p);
                    }
                }
            }
            Cmd::Precharge => {
                let all = p.a & 0x400 != 0;
                for bank in 0..4 {
                    if all || bank == p.ba {
                        self.precharge(n, bank);
                    }
                }
            }
            Cmd::Refresh => {
                for bank in 0..4 {
                    let b = self.banks[bank];
                    if b.row.is_some() {
                        self.err(n, format!("AUTO REFRESH with bank {bank} open (§7.16)"));
                    }
                    if !self.elapsed(n, b.pre_at, self.t.rp_ps) {
                        self.err(n, format!("AUTO REFRESH before tRP of bank {bank}"));
                    }
                    if !self.elapsed(n, b.act_at, self.t.rc_ps) {
                        self.err(n, format!("AUTO REFRESH before tRC of bank {bank}"));
                    }
                }
                if !self.elapsed(n, self.last_ref, self.t.rc_ps) {
                    self.err(n, "AUTO REFRESH before tRC of the last one".into());
                }
                self.last_ref = Some(n);
                self.refs += 1;
                self.ref_times.push(n);
            }
            Cmd::Mode => {
                for bank in 0..4 {
                    let b = self.banks[bank];
                    if b.row.is_some() {
                        self.err(n, format!("MODE REGISTER SET with bank {bank} open (§7.2)"));
                    }
                    if !self.elapsed(n, b.pre_at, self.t.rp_ps) {
                        self.err(n, "MODE REGISTER SET before tRP".into());
                    }
                }
                if !self.elapsed(n, self.last_ref, self.t.rc_ps) {
                    self.err(n, "MODE REGISTER SET before tRC of a refresh".into());
                }
                let want = 0b111 | (cast::<u16>(self.t.cas) << 4);
                if p.a != want || p.ba != 0 {
                    self.err(
                        n,
                        format!("mode {:#06x} on bank {}, wanted {want:#06x}", p.a, p.ba),
                    );
                }
                self.mode = Some(p.a);
                self.last_mrs = Some(n);
            }
        }

        // Rows open too long (tRAS max), and refresh too rare.
        for bank in 0..4 {
            let b = self.banks[bank];
            if b.row.is_some() && !self.elapsed(n, b.act_at, self.t.ras_max_ps + 1) {
                // still inside the limit
            } else if b.row.is_some() {
                self.err(n, format!("bank {bank} open longer than tRAS(max)"));
                self.banks[bank].act_at = Some(n);
            }
        }
        if self.mode.is_some()
            && let Some(r) = self.last_ref
            && (n - r) * self.t.tck_ps > self.t.refi_ps
        {
            self.err(
                n,
                format!("no AUTO REFRESH for {} ns", (n - r) * self.t.tck_ps / 1000),
            );
            self.last_ref = Some(n);
        }

        // The bus: the FPGA drives this cycle from half a cycle before
        // its edge to half a cycle after its end; the part drives a word
        // from its cycle's edge to tHZ after the next.
        if p.oe {
            for k in n.saturating_sub(2)..=n {
                if let Some(o) = self.output(k)
                    && o.masked != 3
                {
                    // Part: half cycles [2k, 2k+3]; FPGA: [2n-1, 2n+2].
                    if 2 * n - 1 <= 2 * k + 3 {
                        self.err(n, format!("bus contention with the read word of cycle {k}"));
                    }
                }
            }
        }
    }

    fn active(&mut self, n: u64, p: Pins) {
        let bank = p.ba;
        if self.mode.is_none() || self.refs < 8 {
            self.err(
                n,
                "ACTIVE before the mode is set and eight refreshes done (§7.1)".into(),
            );
        }
        let b = self.banks[bank];
        if b.row.is_some() {
            self.err(n, format!("ACTIVE to bank {bank}, which is open (§7.3)"));
        }
        if !self.elapsed(n, b.act_at, self.t.rc_ps) {
            self.err(n, format!("ACTIVE to bank {bank} before tRC"));
        }
        if !self.elapsed(n, b.pre_at, self.t.rp_ps) {
            self.err(n, format!("ACTIVE to bank {bank} before tRP"));
        }
        if let Some(a) = self.last_act
            && n - a < self.t.rrd
        {
            self.err(n, format!("ACTIVE {} cycle(s) after another: tRRD", n - a));
        }
        if !self.elapsed(n, self.last_ref, self.t.rc_ps) {
            self.err(n, "ACTIVE before tRC of a refresh".into());
        }
        self.banks[bank].row = Some(p.a & 0x1FFF);
        self.banks[bank].act_at = Some(n);
        self.last_act = Some(n);
    }

    fn precharge(&mut self, n: u64, bank: usize) {
        let b = self.banks[bank];
        if b.row.is_some() {
            if !self.elapsed(n, b.act_at, self.t.ras_ps) {
                self.err(n, format!("PRECHARGE of bank {bank} before tRAS"));
            }
            if let Some(w) = b.wr_at
                && n - w < self.t.wr
            {
                self.err(
                    n,
                    format!(
                        "PRECHARGE of bank {bank} {} cycle(s) after data: tWR",
                        n - w
                    ),
                );
            }
        }
        self.banks[bank].row = None;
        self.banks[bank].pre_at = Some(n);
    }

    /// The `i`th data cycle of burst `b`, which is cycle `n`.
    fn slot(&mut self, n: u64, b: Burst, i: u64, p: Pins) {
        if i >= 16 {
            self.err(
                n,
                "a burst longer than sixteen words (the controller's rule)".into(),
            );
        }
        let col = u64::from(b.col) + i;
        if col >= 512 {
            self.err(
                n,
                "a burst wrapped round its row (the controller's rule)".into(),
            );
        }
        let col = (col % 512) as u16;
        let addr = self.word(b.bank, b.row, col);
        self.words += 1;
        if b.read {
            let k = n + self.t.cas;
            let value = self.read_mem(addr);
            self.outs.insert(k, Out { value, masked: 0 });
        } else {
            if !p.oe {
                self.err(
                    n,
                    "a write data cycle with the bus not driven (the controller's rule)".into(),
                );
            }
            let old = self.read_mem(addr);
            let mut v = old;
            if p.dqm & 1 == 0 {
                v = (v & 0xFF00) | (p.dq & 0x00FF);
            }
            if p.dqm & 2 == 0 {
                v = (v & 0x00FF) | (p.dq & 0xFF00);
            }
            self.mem.insert(addr, v);
            self.banks[b.bank].wr_at = Some(n);
        }
    }

    /// DQM masks read output two cycles later (§5); applied when the
    /// word goes out.
    fn settle_output(&mut self, k: u64) {
        let mask = self.dqm.get(&(k.wrapping_sub(2))).copied().unwrap_or(0);
        if let Some(o) = self.outs.get_mut(&k) {
            o.masked = mask;
        }
    }
}

fn cmd_name(c: Cmd) -> &'static str {
    match c {
        Cmd::Inhibit => "INHIBIT",
        Cmd::Nop => "NOP",
        Cmd::Active => "ACTIVE",
        Cmd::Read => "READ",
        Cmd::Write => "WRITE",
        Cmd::Precharge => "PRECHARGE",
        Cmd::Refresh => "REFRESH",
        Cmd::Mode => "MODE",
        Cmd::Stop => "BURST STOP",
    }
}

// ---------------------------------------------------------------------------
// The model, held to the datasheet by hand-made sequences
// ---------------------------------------------------------------------------

/// A command for the hand-made sequences.
fn pins(cmd: Cmd, ba: usize, a: u16) -> Pins {
    let (cs, ras, cas, we) = match cmd {
        Cmd::Inhibit => (true, true, true, true),
        Cmd::Nop => (false, true, true, true),
        Cmd::Active => (false, false, true, true),
        Cmd::Read => (false, true, false, true),
        Cmd::Write => (false, true, false, false),
        Cmd::Precharge => (false, false, true, false),
        Cmd::Refresh => (false, false, false, true),
        Cmd::Mode => (false, false, false, false),
        Cmd::Stop => (false, true, true, false),
    };
    Pins {
        cke: true,
        cs_n: cs,
        ras_n: ras,
        cas_n: cas,
        we_n: we,
        ba,
        a,
        dqm: 0,
        dq: 0,
        oe: false,
    }
}

/// Runs a sequence of `(cycle, pins)` on a model already initialised,
/// and returns its errors. Cycles not named are NOPs.
fn run_sequence(seq: &[(u64, Pins)]) -> Vec<String> {
    let t = Timing::w9825g6kh6(166, 3);
    let mut m = Sdram::new(t);
    // A legal initialisation, quickly: the pause is the model's to check
    // and is checked by `the_model_catches_a_short_pause`.
    m.t.init_ps = 0;
    let mut n = 1;
    let mut init = vec![pins(Cmd::Precharge, 0, 0x400)];
    for _ in 0..8 {
        init.push(pins(Cmd::Refresh, 0, 0));
    }
    init.push(pins(Cmd::Mode, 0, 0x37));
    for p in init {
        m.step(n, p);
        // tRC (60 ns) is ten cycles at 166 MHz.
        for _ in 0..12 {
            n += 1;
            let mut nop = pins(Cmd::Nop, 0, 0);
            nop.dqm = 3;
            m.step(n, nop);
        }
        n += 1;
    }
    assert!(
        m.errors.is_empty(),
        "the initialisation itself: {:?}",
        m.errors
    );
    let base = n;
    let last = seq.iter().map(|(c, _)| *c).max().unwrap_or(0) + 20;
    let mut it = seq.iter().peekable();
    for c in 0..=last {
        let p = match it.peek() {
            Some((at, p)) if *at == c => {
                let p = *p;
                it.next();
                p
            }
            _ => pins(Cmd::Nop, 0, 0),
        };
        m.step(base + c, p);
        m.settle_output(base + c);
    }
    m.errors
}

fn expect_error(seq: &[(u64, Pins)], what: &str) {
    let errors = run_sequence(seq);
    assert!(
        errors.iter().any(|e| e.contains(what)),
        "expected an error containing `{what}`, got {errors:?}"
    );
}

fn write_word(ba: usize, col: u16) -> Pins {
    let mut p = pins(Cmd::Write, ba, col);
    p.oe = true;
    p
}

#[test]
fn the_model_takes_a_legal_sequence() {
    // ACTIVE, READ at tRCD (3 cycles at 6.02 ns), a second bank's ACTIVE
    // tRRD later, a four-word read ended by BURST STOP, a write, and
    // the precharges at tRAS and tWR.
    let mut wr = write_word(1, 0);
    wr.dq = 0x1234;
    let mut data = pins(Cmd::Nop, 0, 0);
    data.oe = true;
    let seq = [
        (0, pins(Cmd::Active, 0, 5)),
        (2, pins(Cmd::Active, 1, 7)),
        (3, pins(Cmd::Read, 0, 0)),
        (7, pins(Cmd::Stop, 0, 0)),
        (12, wr),
        (13, data),
        (14, pins(Cmd::Stop, 0, 0)),
        (16, pins(Cmd::Precharge, 1, 0)),
        (17, pins(Cmd::Precharge, 0, 0)),
        (20, pins(Cmd::Active, 0, 6)),
    ];
    let errors = run_sequence(&seq);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn the_model_catches_each_timing() {
    // tRCD: READ two cycles (12 ns) after ACTIVE.
    expect_error(
        &[(0, pins(Cmd::Active, 0, 1)), (2, pins(Cmd::Read, 0, 0))],
        "before tRCD",
    );
    // tRRD: two ACTIVEs one cycle apart.
    expect_error(
        &[(0, pins(Cmd::Active, 0, 1)), (1, pins(Cmd::Active, 1, 1))],
        "tRRD",
    );
    // tRAS: PRECHARGE six cycles (36 ns) after ACTIVE.
    expect_error(
        &[
            (0, pins(Cmd::Active, 0, 1)),
            (6, pins(Cmd::Precharge, 0, 0)),
        ],
        "before tRAS",
    );
    // tRP: ACTIVE two cycles after PRECHARGE.
    expect_error(
        &[
            (0, pins(Cmd::Active, 0, 1)),
            (8, pins(Cmd::Precharge, 0, 0)),
            (10, pins(Cmd::Active, 0, 2)),
        ],
        "before tRP",
    );
    // tRC: ACTIVE nine cycles (54 ns) after the last, tRP met.
    expect_error(
        &[
            (0, pins(Cmd::Active, 0, 1)),
            (7, pins(Cmd::Precharge, 0, 0)),
            (9, pins(Cmd::Active, 0, 2)),
        ],
        "before tRC",
    );
    // tWR: PRECHARGE one cycle after the last word written.
    expect_error(
        &[
            (0, pins(Cmd::Active, 0, 1)),
            (3, write_word(0, 0)),
            (4, pins(Cmd::Stop, 0, 0)),
            (8, pins(Cmd::Precharge, 0, 0)),
            (20, pins(Cmd::Active, 2, 1)),
            (23, write_word(2, 0)),
            (24, pins(Cmd::Precharge, 2, 0)),
        ],
        "tWR",
    );
    // tRFC (tRC after a refresh): ACTIVE five cycles after AUTO REFRESH.
    expect_error(
        &[(0, pins(Cmd::Refresh, 0, 0)), (5, pins(Cmd::Active, 0, 1))],
        "tRC of a refresh",
    );
    // tRSC: a command one cycle after MODE REGISTER SET.
    expect_error(
        &[(0, pins(Cmd::Mode, 0, 0x37)), (1, pins(Cmd::Refresh, 0, 0))],
        "tRSC",
    );
}

#[test]
fn the_model_catches_each_illegal_command() {
    expect_error(&[(0, pins(Cmd::Read, 0, 0))], "which is idle");
    expect_error(
        &[(0, pins(Cmd::Active, 0, 1)), (12, pins(Cmd::Active, 0, 1))],
        "which is open",
    );
    expect_error(
        &[(0, pins(Cmd::Active, 0, 1)), (4, pins(Cmd::Read, 0, 0x400))],
        "auto-precharge",
    );
    expect_error(
        &[(0, pins(Cmd::Active, 3, 1)), (12, pins(Cmd::Refresh, 0, 0))],
        "with bank 3 open",
    );
    expect_error(
        &[(0, pins(Cmd::Active, 3, 1)), (12, pins(Cmd::Mode, 0, 0x37))],
        "with bank 3 open",
    );
    expect_error(&[(0, pins(Cmd::Mode, 0, 0x27))], "wanted");
    // A write cycle with the bus left undriven.
    expect_error(
        &[(0, pins(Cmd::Active, 0, 1)), (4, pins(Cmd::Write, 0, 0))],
        "not driven",
    );
    // A burst of seventeen.
    expect_error(
        &[
            (0, pins(Cmd::Active, 0, 1)),
            (4, pins(Cmd::Read, 0, 0)),
            (21, pins(Cmd::Stop, 0, 0)),
        ],
        "sixteen",
    );
    // A burst that wraps round the row.
    expect_error(
        &[
            (0, pins(Cmd::Active, 0, 1)),
            (4, pins(Cmd::Read, 0, 510)),
            (8, pins(Cmd::Stop, 0, 0)),
        ],
        "wrapped",
    );
    // A command with CKE low.
    let mut low = pins(Cmd::Active, 0, 1);
    low.cke = false;
    expect_error(&[(0, low)], "CKE");
}

#[test]
fn the_model_catches_bus_contention() {
    // A read of four at cycle 4 puts words on the bus at 7 to 10 (CAS
    // latency 3); a write at 12 drives from half a cycle before 12,
    // which is still inside tHZ of the word of 10.
    let seq = [
        (0, pins(Cmd::Active, 0, 1)),
        (4, pins(Cmd::Read, 0, 0)),
        (8, pins(Cmd::Stop, 0, 0)),
        (12, write_word(0, 8)),
        (13, pins(Cmd::Stop, 0, 0)),
    ];
    expect_error(&seq, "bus contention");
    // One cycle later it is clear.
    let seq = [
        (0, pins(Cmd::Active, 0, 1)),
        (4, pins(Cmd::Read, 0, 0)),
        (8, pins(Cmd::Stop, 0, 0)),
        (13, write_word(0, 8)),
        (14, pins(Cmd::Stop, 0, 0)),
    ];
    let errors = run_sequence(&seq);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn the_model_catches_a_short_pause_and_a_missed_refresh() {
    let t = Timing::w9825g6kh6(166, 3);
    let mut m = Sdram::new(t);
    let mut nop = pins(Cmd::Nop, 0, 0);
    nop.dqm = 3;
    for n in 0..1000 {
        m.step(n, nop);
    }
    m.step(1000, pins(Cmd::Precharge, 0, 0x400));
    assert!(
        m.errors.iter().any(|e| e.contains("pause")),
        "{:?}",
        m.errors
    );

    let mut m = Sdram::new(t);
    m.t.init_ps = 0;
    let mut n = 0;
    let mut seq = vec![pins(Cmd::Precharge, 0, 0x400)];
    seq.extend(std::iter::repeat_n(pins(Cmd::Refresh, 0, 0), 8));
    seq.push(pins(Cmd::Mode, 0, 0x37));
    for p in seq {
        m.step(n, p);
        n += 12;
    }
    // 7.8125 µs is 1297 cycles of 6.024 ns: 1300 NOPs is too many.
    for k in 0..1300 {
        m.step(n + k, pins(Cmd::Nop, 0, 0));
    }
    assert!(
        m.errors.iter().any(|e| e.contains("no AUTO REFRESH")),
        "{:?}",
        m.errors
    );
}

#[test]
fn the_model_stores_and_masks() {
    // Write two words with the high byte of the second masked, read them
    // back, and see a DQM two cycles before a read word take it off the
    // bus.
    let t = Timing::w9825g6kh6(166, 3);
    let mut m = Sdram::new(t);
    m.t.init_ps = 0;
    let mut n = 0;
    let mut seq = vec![pins(Cmd::Precharge, 0, 0x400)];
    seq.extend(std::iter::repeat_n(pins(Cmd::Refresh, 0, 0), 8));
    seq.push(pins(Cmd::Mode, 0, 0x37));
    for p in seq {
        m.step(n, p);
        n += 12;
    }
    m.step(n, pins(Cmd::Active, 2, 9));
    n += 3;
    let mut w = write_word(2, 4);
    w.dq = 0xAAAA;
    m.step(n, w);
    n += 1;
    let mut w = pins(Cmd::Nop, 0, 0);
    w.oe = true;
    w.dq = 0x5555;
    w.dqm = 2;
    m.step(n, w);
    n += 1;
    m.step(n, pins(Cmd::Stop, 0, 0));
    n += 1;
    let addr = m.word(2, 9, 4);
    assert_eq!(m.read_mem(addr), 0xAAAA);
    assert_eq!(m.read_mem(addr + 1), (initial(addr + 1) & 0xFF00) | 0x55);
    m.step(n, pins(Cmd::Read, 2, 4));
    let mut mask = pins(Cmd::Nop, 0, 0);
    mask.dqm = 3;
    m.step(n + 1, mask);
    m.step(n + 2, pins(Cmd::Stop, 0, 0));
    for k in n..n + 6 {
        m.settle_output(k);
    }
    assert_eq!(
        m.output(n + 3).map(|o| (o.value, o.masked)),
        Some((0xAAAA, 3))
    );
    assert!(m.errors.is_empty(), "{:?}", m.errors);
}

// ---------------------------------------------------------------------------
// The bench: the controller, the model, and clients on every port
// ---------------------------------------------------------------------------

/// One request a client makes: `len` is words minus one.
#[derive(Clone)]
struct Req {
    we: bool,
    addr: u32,
    len: u8,
    data: Vec<(u16, u8)>,
}

/// A client: what it still has to ask, the write data it owes, and the
/// read words it is owed, each with the value it must be.
#[derive(Default)]
struct Port {
    pending: VecDeque<Req>,
    beats: VecDeque<(u16, u8)>,
    expect: VecDeque<(u16, bool, u32)>,
    /// Percent of cycles it holds back a command or a write beat.
    stall_cmd: u64,
    stall_wr: u64,
    /// This cycle's choices, made before the edge.
    v_cmd: bool,
    v_wr: bool,
    issued: u64,
    words_read: u64,
    words_written: u64,
    /// When each read was accepted, for the latency to its first word.
    asked: VecDeque<u64>,
    in_burst: bool,
    worst_latency: u64,
    total_latency: u64,
    reads: u64,
}

impl Port {
    fn busy(&self) -> bool {
        !self.pending.is_empty() || !self.beats.is_empty() || !self.expect.is_empty()
    }
}

/// How the bench is built.
#[derive(Clone)]
struct Config {
    nports: usize,
    mhz: u64,
    cas: u64,
    init_us: u64,
    cal: bool,
    prio: u32,
    /// Cycles from the part's data cycle to the rising and falling
    /// samples the core sees; `None` never delivers that edge.
    rise_at: Option<u64>,
    fall_at: Option<u64>,
    /// The taps at which each edge's sample is right, inclusive.
    rise_taps: (u64, u64),
    fall_taps: (u64, u64),
    seed: u64,
}

impl Config {
    fn new(nports: usize) -> Config {
        Config {
            nports,
            mhz: 166,
            cas: 3,
            init_us: 2,
            cal: false,
            prio: 1 << (nports - 1),
            rise_at: Some(0),
            fall_at: None,
            rise_taps: (0, 31),
            fall_taps: (0, 31),
            seed: 0x1234_5678_9ABC_DEF1,
        }
    }

    fn params(&self) -> Vec<(&'static str, String)> {
        vec![
            ("NPORTS", self.nports.to_string()),
            ("CLK_MHZ", self.mhz.to_string()),
            ("CAS_LATENCY", self.cas.to_string()),
            ("T_INIT_US", self.init_us.to_string()),
            ("CAL_EN", u32::from(self.cal).to_string()),
            ("PRIO", self.prio.to_string()),
        ]
    }
}

struct Nets {
    clk: NetHandle,
    rst_n: NetHandle,
    cmd_valid: NetHandle,
    cmd_ready: NetHandle,
    cmd_we: NetHandle,
    cmd_addr: NetHandle,
    cmd_len: NetHandle,
    wr_valid: NetHandle,
    wr_ready: NetHandle,
    wr_data: NetHandle,
    wr_be: NetHandle,
    rd_valid: NetHandle,
    rd_data: NetHandle,
    rd_last: NetHandle,
    init_done: NetHandle,
    cal_req: NetHandle,
    cal_ok: NetHandle,
    cal_tap: NetHandle,
    cal_edge: NetHandle,
    cal_lat: NetHandle,
    cke: NetHandle,
    cs_n: NetHandle,
    ras_n: NetHandle,
    cas_n: NetHandle,
    we_n: NetHandle,
    ba: NetHandle,
    a: NetHandle,
    dqm: NetHandle,
    dq_o: NetHandle,
    dq_oe: NetHandle,
    dq_rise: NetHandle,
    dq_fall: NetHandle,
    dly_tap: NetHandle,
    dly_ld: NetHandle,
}

const HALF: u64 = 500;

struct Bench<'d> {
    sim: Simulator<'d>,
    n: Nets,
    cfg: Config,
    model: Sdram,
    ports: Vec<Port>,
    shadow: HashMap<u32, u16>,
    rng: Rng,
    /// The core cycle about to be simulated.
    cycle: u64,
    ready: u64,
    wready: u64,
    tap: u64,
    errors: Vec<String>,
}

fn get(sim: &Simulator<'_>, h: NetHandle) -> u64 {
    let v = sim.get(h);
    field(&v, 0, v.width().min(64))
}

impl<'d> Bench<'d> {
    fn new(design: &'d Design, cfg: Config) -> Bench<'d> {
        let sim = Simulator::new(design, SimOptions::default())
            .unwrap_or_else(|d| panic!("cannot simulate: {} problem(s)", d.len()));
        let top = sim.top_name().to_owned();
        let h = |name: &str| {
            sim.net(&format!("{top}.{name}"))
                .unwrap_or_else(|| panic!("no net {name}"))
        };
        let n = Nets {
            clk: h("clk"),
            rst_n: h("rst_n"),
            cmd_valid: h("cmd_valid"),
            cmd_ready: h("cmd_ready"),
            cmd_we: h("cmd_we"),
            cmd_addr: h("cmd_addr"),
            cmd_len: h("cmd_len"),
            wr_valid: h("wr_valid"),
            wr_ready: h("wr_ready"),
            wr_data: h("wr_data"),
            wr_be: h("wr_be"),
            rd_valid: h("rd_valid"),
            rd_data: h("rd_data"),
            rd_last: h("rd_last"),
            init_done: h("init_done"),
            cal_req: h("cal_req"),
            cal_ok: h("cal_ok"),
            cal_tap: h("cal_tap"),
            cal_edge: h("cal_edge"),
            cal_lat: h("cal_lat"),
            cke: h("sdram_cke"),
            cs_n: h("sdram_cs_n"),
            ras_n: h("sdram_ras_n"),
            cas_n: h("sdram_cas_n"),
            we_n: h("sdram_we_n"),
            ba: h("sdram_ba"),
            a: h("sdram_a"),
            dqm: h("sdram_dqm"),
            dq_o: h("sdram_dq_o"),
            dq_oe: h("sdram_dq_oe"),
            dq_rise: h("dq_rise"),
            dq_fall: h("dq_fall"),
            dly_tap: h("dly_tap"),
            dly_ld: h("dly_ld"),
        };
        let mut t = Timing::w9825g6kh6(cfg.mhz, cfg.cas);
        t.init_ps = cfg.init_us * 1_000_000;
        let ports = (0..cfg.nports).map(|_| Port::default()).collect();
        let rng = Rng(cfg.seed);
        let mut b = Bench {
            sim,
            n,
            cfg,
            model: Sdram::new(t),
            ports,
            shadow: HashMap::new(),
            rng,
            cycle: 0,
            ready: 0,
            wready: 0,
            tap: 0,
            errors: Vec::new(),
        };
        b.reset();
        b
    }

    fn reset(&mut self) {
        let np = n32(self.cfg.nports);
        let s = &mut self.sim;
        s.set(self.n.clk, bit(false));
        s.set(self.n.rst_n, bit(false));
        s.set(self.n.cmd_valid, wide(np, &[0]));
        s.set(self.n.cmd_we, wide(np, &[0]));
        s.set(self.n.cmd_addr, wide(np * 24, &[0, 0]));
        s.set(self.n.cmd_len, wide(np * 4, &[0]));
        s.set(self.n.wr_valid, wide(np, &[0]));
        s.set(self.n.wr_data, wide(np * 16, &[0, 0]));
        s.set(self.n.wr_be, wide(np * 2, &[0]));
        s.set(self.n.cal_req, bit(false));
        s.set(self.n.dq_rise, wide(16, &[0]));
        s.set(self.n.dq_fall, wide(16, &[0]));
        s.run_for(HALF);
        for _ in 0..2 {
            s.run_for(HALF);
            s.set(self.n.clk, bit(true));
            s.run_for(HALF);
            s.set(self.n.clk, bit(false));
        }
        s.set(self.n.rst_n, bit(true));
    }

    /// What the core samples on `edge` in core cycle `m`.
    fn sample(&mut self, m: u64, edge: usize) -> u16 {
        let (at, taps) = if edge == 0 {
            (self.cfg.rise_at, self.cfg.rise_taps)
        } else {
            (self.cfg.fall_at, self.cfg.fall_taps)
        };
        let garbage = (self.rng.next() & 0xFFFF) as u16;
        let Some(at) = at else { return garbage };
        if self.tap < taps.0 || self.tap > taps.1 || m < at {
            return garbage;
        }
        match self.model.output(m - at) {
            Some(o) if o.masked == 0 => o.value,
            _ => garbage,
        }
    }

    /// One core cycle: drive the inputs, clock, and hand the pins the
    /// edge produced to the model.
    fn step(&mut self) {
        let m = self.cycle;
        let np = self.cfg.nports;
        let mut valid = 0u64;
        let mut we = 0u64;
        let mut addr = [0u64; 3];
        let mut len = 0u64;
        let mut wvalid = 0u64;
        let mut wdata = [0u64; 2];
        let mut wbe = 0u64;
        for p in 0..np {
            let stall_cmd = self.ports[p].stall_cmd;
            let stall_wr = self.ports[p].stall_wr;
            let hold_cmd = self.rng_chance(stall_cmd);
            let hold_wr = self.rng_chance(stall_wr);
            let port = &mut self.ports[p];
            port.v_cmd = false;
            if let Some(r) = port.pending.front()
                && !hold_cmd
            {
                port.v_cmd = true;
                valid |= 1 << p;
                we |= u64::from(r.we) << p;
                let a = u64::from(r.addr);
                let bit0 = p * 24;
                addr[bit0 / 64] |= a << (bit0 % 64);
                if bit0 % 64 > 40 {
                    addr[bit0 / 64 + 1] |= a >> (64 - bit0 % 64);
                }
                len |= u64::from(r.len) << (4 * p);
            }
            port.v_wr = false;
            if let Some(&(d, be)) = port.beats.front()
                && !hold_wr
            {
                port.v_wr = true;
                wvalid |= 1 << p;
                let bit0 = p * 16;
                wdata[bit0 / 64] |= u64::from(d) << (bit0 % 64);
                wbe |= u64::from(be) << (2 * p);
            }
        }
        let rise = self.sample(m, 0);
        let fall = self.sample(m, 1);
        let np32 = n32(np);
        let s = &mut self.sim;
        s.set(self.n.cmd_valid, wide(np32, &[valid]));
        s.set(self.n.cmd_we, wide(np32, &[we]));
        s.set(self.n.cmd_addr, wide(np32 * 24, &addr));
        s.set(self.n.cmd_len, wide(np32 * 4, &[len]));
        s.set(self.n.wr_valid, wide(np32, &[wvalid]));
        s.set(self.n.wr_data, wide(np32 * 16, &wdata));
        s.set(self.n.wr_be, wide(np32 * 2, &[wbe]));
        s.set(self.n.dq_rise, wide(16, &[u64::from(rise)]));
        s.set(self.n.dq_fall, wide(16, &[u64::from(fall)]));
        s.run_for(HALF);
        s.set(self.n.clk, bit(true));
        s.run_for(HALF);
        s.set(self.n.clk, bit(false));

        // The handshakes that edge completed.
        for p in 0..np {
            if self.ports[p].v_cmd && self.ready >> p & 1 == 1 {
                let r = self.ports[p].pending.pop_front().expect("a request");
                self.accept(p, r, m);
            }
            if self.ports[p].v_wr && self.wready >> p & 1 == 1 {
                self.ports[p].beats.pop_front();
            }
        }
        self.ready = get(&self.sim, self.n.cmd_ready);
        self.wready = get(&self.sim, self.n.wr_ready);

        // Read data, now on the ports.
        let rv = get(&self.sim, self.n.rd_valid);
        if rv != 0 {
            let rl = get(&self.sim, self.n.rd_last);
            let data = self.sim.get(self.n.rd_data);
            for p in 0..np {
                if rv >> p & 1 == 0 {
                    continue;
                }
                let got: u16 = cast(field(&data, 16 * n32(p), 16));
                let last = rl >> p & 1 == 1;
                let port = &mut self.ports[p];
                match port.expect.pop_front() {
                    None => self
                        .errors
                        .push(format!("cycle {m}: port {p} got a word it never asked for")),
                    Some((want, want_last, addr)) => {
                        if got != want || last != want_last {
                            self.errors.push(format!(
                                "cycle {m}: port {p} read {got:#06x}{} at {addr:#08x}, wanted {want:#06x}{}",
                                if last { " (last)" } else { "" },
                                if want_last { " (last)" } else { "" }
                            ));
                        }
                    }
                }
                port.words_read += 1;
                if !port.in_burst
                    && let Some(at) = port.asked.pop_front()
                {
                    let lat = m - at;
                    port.worst_latency = port.worst_latency.max(lat);
                    port.total_latency += lat;
                    port.reads += 1;
                }
                port.in_burst = !last;
            }
        }

        // The pins, for the model.
        let s = &self.sim;
        let pins = Pins {
            cke: get(s, self.n.cke) == 1,
            cs_n: get(s, self.n.cs_n) == 1,
            ras_n: get(s, self.n.ras_n) == 1,
            cas_n: get(s, self.n.cas_n) == 1,
            we_n: get(s, self.n.we_n) == 1,
            ba: cast(get(s, self.n.ba)),
            a: cast(get(s, self.n.a)),
            dqm: cast(get(s, self.n.dqm)),
            dq: cast(get(s, self.n.dq_o)),
            oe: get(s, self.n.dq_oe) == 1,
        };
        if get(s, self.n.dly_ld) == 1 {
            self.tap = get(s, self.n.dly_tap);
        }
        self.model.step(m + 1, pins);
        self.model.settle_output(m + 1);
        self.cycle += 1;
    }

    fn rng_chance(&mut self, percent: u64) -> bool {
        percent != 0 && self.rng.chance(percent)
    }

    /// A request taken: a write's data becomes owed and the shadow
    /// learns it; a read's words become expected.
    fn accept(&mut self, p: usize, r: Req, m: u64) {
        let port = &mut self.ports[p];
        port.issued += 1;
        if r.we {
            for (i, &(d, be)) in r.data.iter().enumerate() {
                let a = r.addr + n32(i);
                let old = *self.shadow.get(&a).unwrap_or(&initial(a));
                let mut v = old;
                if be & 1 != 0 {
                    v = (v & 0xFF00) | (d & 0xFF);
                }
                if be & 2 != 0 {
                    v = (v & 0x00FF) | (d & 0xFF00);
                }
                self.shadow.insert(a, v);
                port.beats.push_back((d, be));
                port.words_written += 1;
            }
        } else {
            for i in 0..=u32::from(r.len) {
                let a = r.addr + i;
                let v = *self.shadow.get(&a).unwrap_or(&initial(a));
                port.expect.push_back((v, i == u32::from(r.len), a));
            }
            port.asked.push_back(m);
        }
    }

    fn init(&mut self, limit: u64) {
        while get(&self.sim, self.n.init_done) == 0 {
            self.step();
            assert!(self.cycle < limit, "init_done never rose");
        }
    }

    fn busy(&self) -> bool {
        self.ports.iter().any(Port::busy)
    }

    /// Runs until every client is served, and returns the cycles taken.
    fn drain(&mut self, limit: u64) -> u64 {
        let start = self.cycle;
        while self.busy() {
            self.step();
            assert!(
                self.cycle - start < limit,
                "the clients were not served in {limit} cycles: {}",
                self.status()
            );
        }
        self.cycle - start
    }

    fn status(&self) -> String {
        self.ports
            .iter()
            .enumerate()
            .map(|(p, port)| {
                format!(
                    "port {p}: {} to ask, {} beats owed, {} words expected",
                    port.pending.len(),
                    port.beats.len(),
                    port.expect.len()
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn check(&self) {
        assert!(
            self.model.errors.is_empty(),
            "the model saw {} violation(s):\n  {}",
            self.model.errors.len(),
            self.model.errors.join("\n  ")
        );
        assert!(
            self.errors.is_empty(),
            "{} wrong read(s):\n  {}",
            self.errors.len(),
            self.errors.join("\n  ")
        );
    }
}

/// A write of `len + 1` words of random data at `addr`.
fn write_req(rng: &mut Rng, addr: u32, len: u8, partial: bool) -> Req {
    let data = (0..=len)
        .map(|_| {
            let be: u8 = if partial { cast(rng.below(3) + 1) } else { 3 };
            ((rng.next() & 0xFFFF) as u16, be)
        })
        .collect();
    Req {
        we: true,
        addr,
        len,
        data,
    }
}

fn read_req(addr: u32, len: u8) -> Req {
    Req {
        we: false,
        addr,
        len,
        data: Vec::new(),
    }
}

/// A random burst inside one row: `len` words minus one, never past
/// the row's end.
fn random_burst(rng: &mut Rng, row_base: u32) -> (u32, u8) {
    let col = rng.below32(512);
    let room = 512 - col;
    let len = (rng.below32(16) + 1).min(room);
    (row_base + col, cast(u64::from(len - 1)))
}

#[test]
fn random_traffic_on_four_ports() {
    let mut cfg = Config::new(4);
    cfg.cal = true;
    let d = design("sdram_burst", &cfg.params(), &[]);
    let mut b = Bench::new(&d, cfg);
    b.init(20_000);
    let mut rng = Rng(77);
    // Each port writes only rows of its own, a few in every bank so that
    // rows collide within a bank, and reads its own rows and the rows
    // nobody writes.
    for p in 0..4usize {
        b.ports[p].stall_cmd = 20;
        b.ports[p].stall_wr = 15;
        for _ in 0..120 {
            let bank = rng.below32(4);
            let r = if rng.chance(25) {
                let shared = rng.below32(6) * 4 + 300;
                let (addr, len) = random_burst(&mut rng, (shared << 11) | (bank << 9));
                read_req(addr, len)
            } else {
                let own = rng.below32(6) * 4 + n32(p) * 32 + 100;
                let (addr, len) = random_burst(&mut rng, (own << 11) | (bank << 9));
                if rng.chance(50) {
                    let partial = rng.chance(30);
                    write_req(&mut rng, addr, len, partial)
                } else {
                    read_req(addr, len)
                }
            };
            b.ports[p].pending.push_back(r);
        }
    }
    let cycles = b.drain(400_000);
    b.check();
    let words: u64 = b.ports.iter().map(|p| p.words_read + p.words_written).sum();
    println!(
        "random traffic: {words} words in {cycles} cycles, {:.1} % of the bus; commands {:?}",
        100.0 * words as f64 / cycles as f64,
        b.model.count
    );
}

// ---------------------------------------------------------------------------
// Bandwidth, counted in cycles
// ---------------------------------------------------------------------------

/// Data cycles on the SDRAM bus over the cycles a drain took.
fn efficiency(b: &mut Bench<'_>, limit: u64) -> (u64, u64) {
    let words0 = b.model.words;
    let cycles = b.drain(limit);
    // The last read words are still in the PHY when the clients are
    // served; nothing is left on the bus.
    (b.model.words - words0, cycles)
}

#[test]
fn sequential_bursts_fill_the_bus() {
    let cfg = Config::new(4);
    let d = design("sdram_burst", &cfg.params(), &[]);
    let mut b = Bench::new(&d, cfg);
    b.init(5_000);
    let mut rng = Rng(5);
    // One client reading 64 KiB in bursts of sixteen.
    for i in 0..2048u32 {
        b.ports[1]
            .pending
            .push_back(read_req(0x10_0000 + i * 16, 15));
    }
    let (words, cycles) = efficiency(&mut b, 100_000);
    let read = words as f64 / cycles as f64;
    // The same, written.
    for i in 0..2048u32 {
        let r = write_req(&mut rng, 0x20_0000 + i * 16, 15, false);
        b.ports[2].pending.push_back(r);
    }
    let (words, cycles) = efficiency(&mut b, 100_000);
    let write = words as f64 / cycles as f64;
    // And four clients streaming at once, two reading and two writing.
    for i in 0..512u32 {
        b.ports[0]
            .pending
            .push_back(read_req(0x30_0000 + i * 16, 15));
        b.ports[1]
            .pending
            .push_back(read_req(0x40_0000 + i * 16, 15));
        let r = write_req(&mut rng, 0x50_0000 + i * 16, 15, false);
        b.ports[2].pending.push_back(r);
        let r = write_req(&mut rng, 0x60_0000 + i * 16, 15, false);
        b.ports[3].pending.push_back(r);
    }
    let (words, cycles) = efficiency(&mut b, 100_000);
    let mixed = words as f64 / cycles as f64;
    b.check();
    println!(
        "sequential bursts of 16: read {:.1} %, write {:.1} %, two readers and two writers {:.1} % \
         of the bus; at 166 MHz that is {:.0}, {:.0} and {:.0} MB/s",
        100.0 * read,
        100.0 * write,
        100.0 * mixed,
        333.0 * read,
        333.0 * write,
        333.0 * mixed
    );
    // Cycles, not seconds: a sixteen-word burst train loses only the
    // refresh and the turn-arounds.
    assert!(
        read > 0.9,
        "sequential reads use {:.1} % of the bus",
        100.0 * read
    );
    assert!(
        write > 0.9,
        "sequential writes use {:.1} % of the bus",
        100.0 * write
    );
}

// ---------------------------------------------------------------------------
// The frame layout
// ---------------------------------------------------------------------------

/// The layout of README.md §"The frame layout", for 1280 x 720: the
/// reference `frame_addr` is held to.
fn frame_addr(frame: u32, plane: u32, x: u32, y: u32) -> (u32, u32) {
    let (tw, th) = (40u32, 23u32);
    let twq = tw.div_ceil(4);
    let (ctw, cth) = (tw.div_ceil(2), th.div_ceil(2));
    let ctwq = ctw.div_ceil(4);
    let luma = th * twq;
    let chroma = cth * ctwq;
    let frame_rows = luma + 2 * chroma;
    let (tx, ty) = (x / 32, y / 32);
    let base = [0, luma, luma + chroma][plane as usize];
    let per = if plane == 0 { twq } else { ctwq };
    let row = frame * frame_rows + base + ty * per + tx / 4;
    let bank = (tx + 2 * ty) % 4;
    let col = (y % 32) * 16 + (x % 32) / 2;
    ((row << 11) | (bank << 9) | col, 16 - (x % 32) / 2)
}

struct FrameAddr<'d> {
    sim: Simulator<'d>,
    frame: NetHandle,
    plane: NetHandle,
    x: NetHandle,
    y: NetHandle,
    addr: NetHandle,
    run: NetHandle,
}

impl FrameAddr<'_> {
    fn eval(&mut self, frame: u32, plane: u32, x: u32, y: u32) -> (u32, u32) {
        self.sim
            .set(self.frame, Logic::from_u64(u64::from(frame), 5));
        self.sim
            .set(self.plane, Logic::from_u64(u64::from(plane), 2));
        self.sim.set(self.x, Logic::from_u64(u64::from(x), 11));
        self.sim.set(self.y, Logic::from_u64(u64::from(y), 11));
        self.sim.run_for(10);
        (
            cast(get(&self.sim, self.addr)),
            cast(get(&self.sim, self.run)),
        )
    }
}

#[test]
fn frame_addr_is_the_documented_layout() {
    let d = design("frame_addr", &[], &[]);
    let sim = Simulator::new(&d, SimOptions::default()).expect("simulates");
    let top = sim.top_name().to_owned();
    let h = |n: &str| sim.net(&format!("{top}.{n}")).expect("a net");
    let mut fa = FrameAddr {
        frame: h("frame"),
        plane: h("plane"),
        x: h("x"),
        y: h("y"),
        addr: h("addr"),
        run: h("run"),
        sim,
    };
    let mut rng = Rng(99);
    let mut rows: HashMap<(u32, u32), (u32, u32, u32, u32)> = HashMap::new();
    let mut checked = 0;
    // Every tile of every plane of the 23 frames that fit, at a pixel of
    // each chosen at random, against the reference; and no two tiles in
    // one row of one bank.
    for frame in 0..23u32 {
        for plane in 0..3u32 {
            let (w, h): (u32, u32) = if plane == 0 { (1280, 720) } else { (640, 360) };
            for ty in 0..h.div_ceil(32) {
                for tx in 0..w.div_ceil(32) {
                    let x = tx * 32 + rng.below32(32);
                    let y = ty * 32 + rng.below32(32);
                    let got = fa.eval(frame, plane, x, y);
                    let want = frame_addr(frame, plane, x, y);
                    assert_eq!(got, want, "frame {frame} plane {plane} ({x}, {y})");
                    let row = got.0 >> 11;
                    let bank = (got.0 >> 9) & 3;
                    assert!(row < 8192, "frame {frame} is past the part");
                    if let Some(other) = rows.insert((bank, row), (frame, plane, tx, ty)) {
                        assert_eq!(
                            other,
                            (frame, plane, tx, ty),
                            "two tiles share bank {bank} row {row}"
                        );
                    }
                    checked += 1;
                }
            }
        }
    }
    // Neighbours, diagonals included, are in other banks.
    for ty in 0..23u32 {
        for tx in 0..40u32 {
            let bank = |tx: u32, ty: u32| (frame_addr(0, 0, tx * 32, ty * 32).0 >> 9) & 3;
            for (dx, dy) in [(1i32, 0i32), (0, 1), (1, 1), (1, -1)] {
                let (nx, ny) = (tx as i32 + dx, ty as i32 + dy);
                if (0..40).contains(&nx) && (0..23).contains(&ny) {
                    assert_ne!(bank(tx, ty), bank(nx.unsigned_abs(), ny.unsigned_abs()));
                }
            }
        }
    }
    // Inside a tile: the word, and the run to the tile's right edge.
    for _ in 0..2000 {
        let (frame, plane) = (rng.below32(23), rng.below32(3));
        let (x, y) = (rng.below32(2048), rng.below32(2048));
        let got = fa.eval(frame, plane, x, y);
        let want = frame_addr(frame, plane, x, y);
        assert_eq!(got.0 & 0x7FF, want.0 & 0x7FF, "({x}, {y})");
        assert_eq!(got.1, want.1, "({x}, {y})");
    }
    println!("frame_addr: {checked} tiles, 23 frames of 1280 x 720, every row distinct");
}

/// The bursts a 2D fetch of `w` x `h` luma pixels at (`x`, `y`) needs:
/// one per tile per line, each never past its tile's line.
fn block_bursts(frame: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<(u32, u8)> {
    let mut out = Vec::new();
    for line in y..y + h {
        let mut wx = x / 2;
        let end = (x + w - 1) / 2;
        while wx <= end {
            let (addr, run) = frame_addr(frame, 0, wx * 2, line);
            let n = run.min(end - wx + 1);
            out.push((addr, cast(u64::from(n - 1))));
            wx += n;
        }
    }
    out
}

#[test]
fn motion_compensation_fetches() {
    let cfg = Config::new(4);
    let d = design("sdram_burst", &cfg.params(), &[]);
    let mut b = Bench::new(&d, cfg);
    b.init(5_000);
    let mut rng = Rng(31);
    let mut report = Vec::new();
    for (w, h) in [(9u32, 9u32), (13, 13), (21, 21)] {
        let mut words_wanted = 0u64;
        for _ in 0..300 {
            let frame = rng.below32(3);
            let x = rng.below32(1280 - w);
            let y = rng.below32(720 - h);
            for (addr, len) in block_bursts(frame, x, y, w, h) {
                words_wanted += u64::from(len) + 1;
                b.ports[1].pending.push_back(read_req(addr, len));
            }
        }
        let (words, cycles) = efficiency(&mut b, 200_000);
        assert_eq!(words, words_wanted);
        let e = words as f64 / cycles as f64;
        report.push(format!(
            "{w}x{h}: {:.1} % ({:.0} MB/s at 166 MHz)",
            100.0 * e,
            333.0 * e
        ));
        assert!(e > 0.5, "{w}x{h} blocks use {:.1} % of the bus", 100.0 * e);
    }
    // A decoder's mix: port 1 fetches 9 x 9 blocks of frames 0 to 2
    // (sixteen to a macroblock, the worst case of 4 x 4 partitions with
    // fractional vectors), port 2 writes the reconstructed macroblocks of
    // frame 3 in order (sixteen lines of eight words), port 3 reads frame
    // 3's luma for a display, sixteen words at a time, and port 0 writes
    // the bitstream in bursts of sixteen.
    let mut words_wanted = 0u64;
    for mb in 0..60u32 {
        let (mx, my) = (mb % 80 * 16, mb / 80 * 16);
        for _ in 0..16 {
            let frame = rng.below32(3);
            let x = rng.below32(1280 - 9);
            let y = rng.below32(720 - 9);
            for (addr, len) in block_bursts(frame, x, y, 9, 9) {
                words_wanted += u64::from(len) + 1;
                b.ports[1].pending.push_back(read_req(addr, len));
            }
        }
        for line in 0..16 {
            let (addr, _) = frame_addr(3, 0, mx, my + line);
            words_wanted += 8;
            let r = write_req(&mut rng, addr, 7, false);
            b.ports[2].pending.push_back(r);
        }
        for i in 0..4 {
            let (addr, _) = frame_addr(4, 0, (mb * 4 + i) % 40 * 32, 0);
            words_wanted += 16;
            b.ports[3].pending.push_back(read_req(addr, 15));
        }
        words_wanted += 16;
        let r = write_req(&mut rng, (6000 << 11) + mb * 16, 15, false);
        b.ports[0].pending.push_back(r);
    }
    let (words, cycles) = efficiency(&mut b, 400_000);
    assert_eq!(words, words_wanted);
    let mix = words as f64 / cycles as f64;
    b.check();
    println!(
        "motion compensation, 300 random blocks each, one client: {}; a decoder's mix on four \
         ports: {:.1} % ({:.0} MB/s at 166 MHz)",
        report.join(", "),
        100.0 * mix,
        333.0 * mix
    );
    assert!(
        mix > 0.5,
        "the decoder's mix uses {:.1} % of the bus",
        100.0 * mix
    );
}

// ---------------------------------------------------------------------------
// Calibration, priority, the real power-up pause, other clocks
// ---------------------------------------------------------------------------

/// Random reads and writes from every port over rows of its own, every
/// read checked.
fn random_load(b: &mut Bench<'_>, seed: u64, per_port: usize) {
    let mut rng = Rng(seed);
    let n = b.ports.len();
    for p in 0..n {
        for _ in 0..per_port {
            let own = rng.below32(4) * 4 + n32(p) * 16 + 500;
            let bank = rng.below32(4);
            let (addr, len) = random_burst(&mut rng, (own << 11) | (bank << 9));
            let r = if rng.chance(50) {
                write_req(&mut rng, addr, len, false)
            } else {
                read_req(addr, len)
            };
            b.ports[p].pending.push_back(r);
        }
    }
}

fn cal_result(b: &Bench<'_>) -> (bool, u64, u64, u64) {
    (
        get(&b.sim, b.n.cal_ok) == 1,
        get(&b.sim, b.n.cal_edge),
        get(&b.sim, b.n.cal_lat),
        get(&b.sim, b.n.cal_tap),
    )
}

#[test]
fn calibration_finds_the_middle_of_the_longest_window() {
    // Rising-edge samples a cycle late and right at taps 10 to 25;
    // falling-edge samples on time and right at taps 0 to 8. The longest
    // run is the rising edge's, sixteen taps; a cycle late is latency 0
    // where on time is 1 (PHY_RD_LAT is 4); the run's middle is tap 17.
    let mut cfg = Config::new(2);
    cfg.cal = true;
    cfg.rise_at = Some(1);
    cfg.rise_taps = (10, 25);
    cfg.fall_at = Some(0);
    cfg.fall_taps = (0, 8);
    let d = design("sdram_burst", &cfg.params(), &[]);
    let mut b = Bench::new(&d, cfg);
    b.init(20_000);
    assert_eq!(cal_result(&b), (true, 0, 0, 17));
    assert_eq!(b.tap, 17, "the delay was not loaded with the tap chosen");
    random_load(&mut b, 3, 40);
    b.drain(100_000);
    b.check();

    // The board warms up: now the falling edge is right at taps 3 to 30,
    // and the rising edge nowhere. Calibrating again while the clients
    // are idle finds it.
    b.cfg.rise_taps = (40, 40);
    b.cfg.fall_taps = (3, 30);
    b.sim.set(b.n.cal_req, bit(true));
    b.step();
    b.sim.set(b.n.cal_req, bit(false));
    let start = b.cycle;
    while get(&b.sim, b.n.init_done) == 0 || b.cycle < start + 4 {
        b.step();
        assert!(
            b.cycle < start + 20_000,
            "the second calibration never ended"
        );
    }
    assert_eq!(cal_result(&b), (true, 1, 1, 16));
    random_load(&mut b, 4, 40);
    b.drain(100_000);
    b.check();
}

#[test]
fn calibration_with_no_window_keeps_the_defaults() {
    let mut cfg = Config::new(1);
    cfg.cal = true;
    cfg.rise_at = None;
    cfg.fall_at = None;
    let d = design("sdram_burst", &cfg.params(), &[]);
    let mut b = Bench::new(&d, cfg);
    b.init(20_000);
    // DEF_EDGE 0, DEF_LAT 1, DEF_TAP 0, and the failure said.
    assert_eq!(cal_result(&b), (false, 0, 1, 0));
    b.check();
}

/// Ports 0 to 2 keep the controller busy with random sixteen-word reads
/// and writes; port 3 asks for a sixteen-word read every `pace` cycles,
/// as a display would. Returns the display's worst and mean latency, in
/// cycles from its request being taken to its first word.
fn display_latency(prio: u32, pace: u64) -> (u64, f64, u64) {
    let mut cfg = Config::new(4);
    cfg.prio = prio;
    let d = design("sdram_burst", &cfg.params(), &[]);
    let mut b = Bench::new(&d, cfg);
    b.init(5_000);
    let mut rng = Rng(11);
    for p in 0..3usize {
        for _ in 0..400 {
            let row = rng.below32(64) + 1000;
            let addr = (row << 11) | ((rng.below32(4)) << 9) | (rng.below32(32) * 16);
            let r = if rng.chance(50) {
                write_req(&mut rng, addr, 15, false)
            } else {
                read_req(addr, 15)
            };
            b.ports[p].pending.push_back(r);
        }
    }
    let mut next = b.cycle;
    let mut line = 0u32;
    let mut asked = 0;
    while b.ports[..3].iter().any(Port::busy) {
        if b.cycle >= next && b.ports[3].pending.is_empty() {
            b.ports[3]
                .pending
                .push_back(read_req(0x60_0000 + line * 16, 15));
            line += 1;
            asked += 1;
            next = b.cycle + pace;
        }
        b.step();
        assert!(b.cycle < 400_000, "the load never finished");
    }
    b.drain(10_000);
    b.check();
    let port = &b.ports[3];
    assert_eq!(port.reads, asked);
    (
        port.worst_latency,
        port.total_latency as f64 / port.reads as f64,
        port.reads,
    )
}

#[test]
fn the_display_port_is_served_first() {
    // A burst of sixteen every 64 cycles is 25 % of the bus, twice what
    // 1280 x 720 at 30 frames a second in 4:2:0 needs (41 MB/s).
    let (worst, mean, n) = display_latency(0b1000, 64);
    let (worst_rr, mean_rr, _) = display_latency(0, 64);
    println!(
        "display port, {n} reads of 16 under load: worst {worst} cycles, mean {mean:.1} with \
         priority; worst {worst_rr}, mean {mean_rr:.1} without"
    );
    // The bound: eight queue entries of sixteen words ahead of it, each
    // with a bank to open, a direction change and a refresh.
    assert!(worst < 300, "the display waited {worst} cycles");
    assert!(
        mean < mean_rr,
        "priority did not help: {mean:.1} against {mean_rr:.1}"
    );
}

#[test]
fn the_power_up_pause_is_the_datasheets() {
    // The full 200 µs, against a model that wants 200 µs.
    let mut cfg = Config::new(1);
    cfg.init_us = 200;
    let d = design("sdram_burst", &cfg.params(), &[]);
    let mut b = Bench::new(&d, cfg);
    b.init(40_000);
    random_load(&mut b, 9, 30);
    b.drain(50_000);
    b.check();
    let first = b.model.ref_times[0];
    println!(
        "power-up: first refresh at cycle {first}, {} µs at 166 MHz; init_done at cycle {}",
        first * 6024 / 1_000_000,
        b.cycle
    );
}

#[test]
fn other_clocks_and_cas_latencies() {
    // 133 MHz at CAS latency 3, and 133 MHz at CAS latency 2 (the -6
    // part's other speed, §3), each against a model at that clock.
    for (mhz, cas) in [(133, 3), (133, 2), (100, 2)] {
        let mut cfg = Config::new(4);
        cfg.mhz = mhz;
        cfg.cas = cas;
        cfg.cal = true;
        let d = design("sdram_burst", &cfg.params(), &[]);
        let mut b = Bench::new(&d, cfg);
        b.init(20_000);
        random_load(&mut b, mhz + cas, 60);
        let (words, cycles) = efficiency(&mut b, 200_000);
        b.check();
        println!(
            "{mhz} MHz, CAS latency {cas}: random load {:.1} % of the bus",
            100.0 * words as f64 / cycles as f64
        );
    }
}

// ---------------------------------------------------------------------------
// The 7-series pins, with Xilinx's primitives modelled and a board in time
// ---------------------------------------------------------------------------

/// A bench around `sdram_burst_xc7`: its DQ pins are a wire the test
/// drives too, through `ext_dq` and `ext_oe`.
const PHY_TB: &str = "
module phy_tb (
    input  wire        clk,
    input  wire        clk_ref,
    input  wire        rst_n,
    input  wire        ref_rst,
    input  wire [15:0] ext_dq,
    input  wire        ext_oe,
    input  wire        cmd_valid,
    output wire        cmd_ready,
    input  wire        cmd_we,
    input  wire [23:0] cmd_addr,
    input  wire [3:0]  cmd_len,
    input  wire        wr_valid,
    output wire        wr_ready,
    input  wire [15:0] wr_data,
    output wire        rd_valid,
    output wire [15:0] rd_data,
    output wire        rd_last,
    output wire        init_done,
    output wire        cal_ok,
    output wire [4:0]  cal_tap,
    output wire        cal_edge,
    output wire [1:0]  cal_lat,
    output wire        dly_rdy,
    output wire        sdram_clk,
    output wire        sdram_cke,
    output wire        sdram_cs_n,
    output wire        sdram_ras_n,
    output wire        sdram_cas_n,
    output wire        sdram_we_n,
    output wire [1:0]  sdram_ba,
    output wire [12:0] sdram_a,
    output wire [1:0]  sdram_dqm,
    output wire [15:0] dq
);
    assign dq = ext_oe ? ext_dq : 16'bz;
    wire cal_busy;
    sdram_burst_xc7 #(.NPORTS(1), .PRIO(0), .T_INIT_US(1), .CLK_PS(6000)) dut (
        .clk(clk), .clk_ref(clk_ref), .rst_n(rst_n), .ref_rst(ref_rst), .dly_rdy(dly_rdy),
        .cmd_valid(cmd_valid), .cmd_ready(cmd_ready), .cmd_we(cmd_we),
        .cmd_addr(cmd_addr), .cmd_len(cmd_len),
        .wr_valid(wr_valid), .wr_ready(wr_ready), .wr_data(wr_data), .wr_be(2'b11),
        .rd_valid(rd_valid), .rd_data(rd_data), .rd_last(rd_last),
        .init_done(init_done), .cal_req(1'b0), .cal_busy(cal_busy), .cal_ok(cal_ok),
        .cal_tap(cal_tap), .cal_edge(cal_edge), .cal_lat(cal_lat),
        .sdram_clk(sdram_clk), .sdram_cke(sdram_cke), .sdram_cs_n(sdram_cs_n),
        .sdram_ras_n(sdram_ras_n), .sdram_cas_n(sdram_cas_n), .sdram_we_n(sdram_we_n),
        .sdram_ba(sdram_ba), .sdram_a(sdram_a), .sdram_dqm(sdram_dqm), .sdram_dq(dq)
    );
endmodule
";

/// The board, in picoseconds. The figures are DS181's at grade -2
/// (LVCMOS33, 12 mA, fast: TOCKQ 0.49 and TIOOP 2.68 out, TIOPI 1.34 in)
/// and the W9825G6KH-6's (tAC 5, tOH 3); `trace` is the board's own, each
/// way, and `clock_insertion` the global clock's delay to the IO logic,
/// which the data path does not have. None of them is measured.
#[derive(Clone, Copy)]
struct Board {
    period: u64,
    out: u64,
    trace: u64,
    inp: u64,
    clock_insertion: u64,
    tac: u64,
    toh: u64,
}

impl Board {
    fn nominal(trace: u64) -> Board {
        Board {
            period: 6000,
            out: 490 + 2680,
            trace,
            inp: 1340,
            clock_insertion: 1500,
            tac: 5000,
            toh: 3000,
        }
    }

    /// When the part sees the clock edge for the command launched at the
    /// FPGA's rising edge `n` (its forwarded clock rises half a cycle
    /// later), in the simulation's time, where the FPGA's edges are at
    /// `n * period` and its IO logic clocks at those instants.
    fn sdram_edge(&self, n: u64) -> u64 {
        n * self.period + self.period / 2 + self.out + self.trace
    }

    /// The same instant moved to where the FPGA's input sees it: the way
    /// back, less the clock insertion delay the IO logic's clock has and
    /// the simulation leaves out.
    fn back(&self, t: u64) -> u64 {
        (t + self.trace + self.inp).saturating_sub(self.clock_insertion)
    }
}

#[test]
fn the_xc7_pins_capture_reads_after_calibration() {
    let mut names = vec![
        "ODDR".to_owned(),
        "IDDR".to_owned(),
        "IDELAYE2".to_owned(),
        "IDELAYCTRL".to_owned(),
    ];
    names.sort();
    let Some(lib) = reticle::fpga::simulation_source("xc7", &names) else {
        panic!("no simulation models of the 7-series primitives");
    };
    let extra = vec![
        ("xc7_sim.v".to_owned(), lib),
        ("phy_tb.v".to_owned(), PHY_TB.to_owned()),
    ];
    let d = design("phy_tb", &[], &extra);
    for trace in [300u64, 900] {
        let board = Board::nominal(trace);
        let (cal, checked) = run_phy(&d, board);
        println!(
            "xc7 pins, board trace {trace} ps each way: calibrated to edge {}, latency {}, tap {} \
             ({} ps); {checked} words read back right",
            cal.1,
            cal.2,
            cal.3,
            cal.3 * 78
        );
        assert!(cal.0, "calibration found no window");
    }
}

/// Runs the PHY bench on `board`: initialise, calibrate, write and read
/// back. Returns the calibration and how many words were checked.
fn run_phy(d: &Design, board: Board) -> ((bool, u64, u64, u64), u64) {
    let mut sim = Simulator::new(d, SimOptions::default()).expect("simulates");
    let top = sim.top_name().to_owned();
    let h = |n: &str| {
        sim.net(&format!("{top}.{n}"))
            .unwrap_or_else(|| panic!("no {n}"))
    };
    let (clk, clk_ref, rst_n, ref_rst) = (h("clk"), h("clk_ref"), h("rst_n"), h("ref_rst"));
    let (ext_dq, ext_oe) = (h("ext_dq"), h("ext_oe"));
    let (cmd_valid, cmd_ready, cmd_we, cmd_addr, cmd_len) = (
        h("cmd_valid"),
        h("cmd_ready"),
        h("cmd_we"),
        h("cmd_addr"),
        h("cmd_len"),
    );
    let (wr_valid, wr_ready, wr_data) = (h("wr_valid"), h("wr_ready"), h("wr_data"));
    let (rd_valid, rd_data, rd_last) = (h("rd_valid"), h("rd_data"), h("rd_last"));
    let (init_done, cal_ok, cal_tap, cal_edge, cal_lat) = (
        h("init_done"),
        h("cal_ok"),
        h("cal_tap"),
        h("cal_edge"),
        h("cal_lat"),
    );
    let pin = [
        h("sdram_cke"),
        h("sdram_cs_n"),
        h("sdram_ras_n"),
        h("sdram_cas_n"),
        h("sdram_we_n"),
        h("sdram_ba"),
        h("sdram_a"),
        h("sdram_dqm"),
        h("dq"),
    ];
    let dq_t = sim
        .net(&format!("{top}.dut.phy.dq_t"))
        .expect("the PHY's three-state register");
    let mut t = Timing::w9825g6kh6(166, 3);
    t.tck_ps = board.period;
    t.init_ps = 1_000_000;
    let mut model = Sdram::new(t);

    for (n, v) in [
        (clk, bit(false)),
        (clk_ref, bit(false)),
        (rst_n, bit(false)),
        (ref_rst, bit(true)),
        (ext_oe, bit(false)),
        (ext_dq, Logic::zero(16)),
        (cmd_valid, bit(false)),
        (cmd_we, bit(false)),
        (cmd_addr, Logic::zero(24)),
        (cmd_len, Logic::zero(4)),
        (wr_valid, bit(false)),
        (wr_data, Logic::zero(16)),
    ] {
        sim.set(n, v);
    }
    // The models' global reset lasts 100 ns; the IDELAYCTRL wants its
    // reference clock, 200 MHz, for two edges after its reset.
    sim.run_until(150_000);
    sim.set(ref_rst, bit(false));
    let mut now = 150_000;
    for _ in 0..8 {
        now += 2500;
        sim.run_until(now);
        sim.set(clk_ref, bit(true));
        now += 2500;
        sim.run_until(now);
        sim.set(clk_ref, bit(false));
    }
    let base = now.div_ceil(board.period) + 1;

    // The test's port: what to ask, the data it owes, the words it is owed.
    let mut rng = Rng(21);
    let mut pending: VecDeque<Req> = VecDeque::new();
    for i in 0..24u32 {
        let addr = (i % 3 + 200) << 11 | (i % 4) << 9 | (i * 16 % 480);
        pending.push_back(write_req(&mut rng, addr, 15, false));
        pending.push_back(read_req(addr, 15));
    }
    let mut shadow: HashMap<u32, u16> = HashMap::new();
    let mut beats: VecDeque<u16> = VecDeque::new();
    let mut expect: VecDeque<(u16, bool)> = VecDeque::new();
    let mut ready = false;
    let mut wready = false;
    let mut checked = 0u64;
    let mut errors: Vec<String> = Vec::new();
    // The part's drive of DQ, as (time, value) changes, in the
    // simulation's time at the FPGA's input.
    let mut drive: BTreeMap<u64, Option<Logic>> = BTreeMap::new();
    let mut started = false;

    for n in base.. {
        let t0 = n * board.period;
        // Rising edge at t0, falling at t0 + period / 2, with the part's
        // DQ changes in between.
        let mut edges = vec![(t0, true), (t0 + board.period / 2, false)];
        edges.sort();
        let mut go = init_ready(&sim, init_done);
        if go && !started {
            started = true;
        }
        // The inputs for this cycle, set just before its edge.
        let v_cmd = started && !pending.is_empty();
        if let Some(r) = pending.front().filter(|_| v_cmd) {
            sim.set(cmd_valid, bit(true));
            sim.set(cmd_we, bit(r.we));
            sim.set(cmd_addr, Logic::from_u64(u64::from(r.addr), 24));
            sim.set(cmd_len, Logic::from_u64(u64::from(r.len), 4));
        } else {
            sim.set(cmd_valid, bit(false));
        }
        let v_wr = !beats.is_empty();
        sim.set(wr_valid, bit(v_wr));
        sim.set(
            wr_data,
            Logic::from_u64(u64::from(*beats.front().unwrap_or(&0)), 16),
        );
        for (at, rise) in edges {
            // The part's drive up to this edge.
            let due: Vec<u64> = drive.range(..=at).map(|(k, _)| *k).collect();
            for k in due {
                let v = drive.remove(&k).expect("an entry");
                sim.run_until(k);
                match v {
                    Some(l) => {
                        sim.set(ext_dq, l);
                        sim.set(ext_oe, bit(true));
                    }
                    None => sim.set(ext_oe, bit(false)),
                }
            }
            sim.run_until(at);
            sim.set(clk, bit(rise));
            if at == t0 {
                sim.set(rst_n, bit(true));
            }
        }
        // The handshakes of the rising edge.
        if v_cmd && ready {
            let r = pending.pop_front().expect("a request");
            if r.we {
                for (i, &(dw, _)) in r.data.iter().enumerate() {
                    shadow.insert(r.addr + n32(i), dw);
                    beats.push_back(dw);
                }
            } else {
                for i in 0..=u32::from(r.len) {
                    let a = r.addr + i;
                    expect.push_back((
                        *shadow.get(&a).unwrap_or(&initial(a)),
                        i == u32::from(r.len),
                    ));
                }
            }
        }
        if v_wr && wready {
            beats.pop_front();
        }
        // A quarter of the way in, after both edges' changes: the pins
        // the IO logic put out at this cycle's rising edge.
        sim.run_until(t0 + board.period * 3 / 4);
        ready = get(&sim, cmd_ready) == 1;
        wready = get(&sim, wr_ready) == 1;
        if get(&sim, rd_valid) == 1 {
            let got: u16 = cast(get(&sim, rd_data));
            let last = get(&sim, rd_last) == 1;
            match expect.pop_front() {
                Some((want, wl)) if want == got && wl == last => checked += 1,
                Some((want, _)) => {
                    errors.push(format!("cycle {n}: read {got:#06x}, wanted {want:#06x}"))
                }
                None => errors.push(format!("cycle {n}: a word nobody asked for")),
            }
        }
        let value = |i: usize| sim.get(pin[i]);
        let dqv = value(8);
        let p = Pins {
            cke: get(&sim, pin[0]) == 1,
            cs_n: get(&sim, pin[1]) == 1,
            ras_n: get(&sim, pin[2]) == 1,
            cas_n: get(&sim, pin[3]) == 1,
            we_n: get(&sim, pin[4]) == 1,
            ba: cast(get(&sim, pin[5])),
            a: cast(get(&sim, pin[6])),
            dqm: cast(get(&sim, pin[7])),
            dq: cast(dqv.to_u64().unwrap_or(0) & 0xFFFF),
            oe: field(&sim.get(dq_t), 0, 1) == 0,
        };
        model.step(n, p);
        model.settle_output(n);
        // A word the part drives in cycle k: unknown from tOH after the
        // edge before, the word from tAC after it, until tOH after its own.
        let k = n + t.cas;
        if let Some(o) = model.output(k)
            && o.masked == 0
        {
            let prev = board.sdram_edge(k - 1);
            drive.insert(board.back(prev + board.toh), Some(Logic::x(16)));
            drive.insert(
                board.back(prev + board.tac),
                Some(Logic::from_u64(u64::from(o.value), 16)),
            );
            let end = board.back(board.sdram_edge(k) + board.toh);
            drive.entry(end).or_insert(None);
        }
        go = go && pending.is_empty() && beats.is_empty() && expect.is_empty();
        if go || n > base + 60_000 {
            break;
        }
    }
    assert!(
        model.errors.is_empty(),
        "the model saw {} violation(s):\n  {}",
        model.errors.len(),
        model.errors.join("\n  ")
    );
    assert!(
        errors.is_empty(),
        "{} wrong read(s):\n  {}",
        errors.len(),
        errors.join("\n  ")
    );
    assert!(
        pending.is_empty() && expect.is_empty(),
        "the port was not served"
    );
    (
        (
            get(&sim, cal_ok) == 1,
            get(&sim, cal_edge),
            get(&sim, cal_lat),
            get(&sim, cal_tap),
        ),
        checked,
    )
}

fn init_ready(sim: &Simulator<'_>, init_done: NetHandle) -> bool {
    sim.get(init_done).to_u64() == Some(1)
}
