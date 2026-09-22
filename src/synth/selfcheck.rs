//! Behavioural self-check of the golden designs (test only).
//!
//! A tiny cycle-based interpreter over the IR, good enough to compare a
//! module before and after synthesis: it runs processes as statement
//! interpreters and cells by their primitive semantics, settles the
//! combinational logic to a fixpoint, and steps clocks. It exists because
//! the real simulator is developed concurrently; it is deliberately
//! simple (no delays, no event ordering between processes) and lives
//! under `#[cfg(test)]` only.
//!
//! The check drives every golden design in `testdata/synth/` with random
//! two-state inputs (a few reset cycles first for sequential designs) and
//! asserts that every output bit that is known before synthesis has the
//! same value after it. Unknown (`x`) bits of the reference are don't-care.

use std::collections::HashMap;

use crate::diag::Diagnostics;
use crate::ir::{
    AssignKind, Block, CaseKind, CellKind, Const, Design, Edge, ExprId, Lvalue, MemoryId, Module,
    NetId, Polarity, PortDir, ProcessKind, Reset, StmtKind,
};
use crate::logic::Bit;
use crate::synth::eval::{Env, eval};
use crate::synth::{SynthOptions, run};

/// Simulation state of one module.
struct Sim<'a> {
    m: &'a Module,
    nets: Vec<Const>,
    mems: Vec<Vec<Const>>,
}

impl Env for Sim<'_> {
    fn net(&mut self, net: NetId) -> Option<Const> {
        Some(self.nets[net.index()].clone())
    }

    fn mem(&mut self, mem: MemoryId, addr: &Const) -> Option<Const> {
        let width = self.m.memories[mem].elem.width()?;
        Some(
            addr.to_u64()
                .and_then(|a| self.mems[mem.index()].get(usize::try_from(a).ok()?))
                .cloned()
                .unwrap_or_else(|| Const::x(width)),
        )
    }
}

enum Flow {
    Next,
    Break,
    Continue,
}

/// A deferred (non-blocking) update, with its address already sampled.
enum Update {
    Net(Lvalue, Const),
    Mem(MemoryId, Const, Const),
}

impl<'a> Sim<'a> {
    fn new(m: &'a Module) -> Self {
        let nets = m
            .nets
            .values()
            .map(|n| match n.attrs.get("init") {
                Some(crate::ir::AttrValue::Const(c)) => c.clone(),
                _ => Const::x(n.ty.width().unwrap_or(0)),
            })
            .collect();
        let mems = m
            .memories
            .values()
            .map(|mem| {
                let w = mem.elem.width().unwrap_or(0);
                let mut v = vec![Const::x(w); usize::try_from(mem.size).unwrap_or(0)];
                if let Some(init) = &mem.init {
                    for (slot, c) in v.iter_mut().zip(init) {
                        *slot = c.clone();
                    }
                }
                v
            })
            .collect();
        let mut sim = Sim { m, nets, mems };
        for (_, p) in m.processes.iter() {
            if p.kind == ProcessKind::Initial {
                let mut nba = Vec::new();
                sim.exec_block(&p.body, &mut nba);
                sim.commit(nba);
            }
        }
        sim
    }

    fn value(&mut self, id: ExprId) -> Const {
        let width = self.m.expr(id).ty.width().unwrap_or(0);
        eval(self.m, id, self).unwrap_or_else(|| Const::x(width))
    }

    fn truth(&mut self, id: ExprId) -> Bit {
        self.value(id).truth()
    }

    fn set_bits(&mut self, net: NetId, lo: u32, v: &Const) {
        let cur = &mut self.nets[net.index()];
        for i in 0..v.width() {
            if lo + i < cur.width() {
                cur.set_bit(lo + i, v.bit(i));
            }
        }
    }

    fn write(&mut self, lv: &Lvalue, v: Const) {
        match lv {
            Lvalue::Net(n) => {
                let w = self.nets[n.index()].width();
                let signed = self.nets[n.index()].is_signed();
                self.nets[n.index()] = v.resize(w).with_signed(signed);
            }
            Lvalue::Slice { net, lo, .. } => self.set_bits(*net, *lo, &v),
            Lvalue::Index { net, index } => {
                if let Some(i) = self.value(*index).to_u64()
                    && let Ok(i) = u32::try_from(i)
                {
                    self.set_bits(*net, i, &v.resize(1));
                }
            }
            Lvalue::Concat(parts) => {
                let mut hi = v.width();
                for part in parts {
                    let w = self.lvalue_width(part);
                    if w == 0 {
                        continue;
                    }
                    let piece = v.slice(hi - 1, hi - w);
                    self.write(part, piece);
                    hi -= w;
                }
            }
            Lvalue::MemElem { mem, addr } => {
                let a = self.value(*addr);
                self.mem_write(*mem, &a, v);
            }
        }
    }

    fn lvalue_width(&self, lv: &Lvalue) -> u32 {
        match lv {
            Lvalue::Net(n) => self.nets[n.index()].width(),
            Lvalue::Slice { hi, lo, .. } => hi - lo + 1,
            Lvalue::Index { .. } => 1,
            Lvalue::Concat(parts) => parts.iter().map(|p| self.lvalue_width(p)).sum(),
            Lvalue::MemElem { mem, .. } => self.m.memories[*mem].elem.width().unwrap_or(0),
        }
    }

    fn mem_write(&mut self, mem: MemoryId, addr: &Const, v: Const) {
        if let Some(a) = addr.to_u64()
            && let Some(slot) =
                self.mems[mem.index()].get_mut(usize::try_from(a).unwrap_or(usize::MAX))
        {
            *slot = v.resize(slot.width());
        }
    }

    fn commit(&mut self, nba: Vec<Update>) {
        for u in nba {
            match u {
                Update::Net(lv, v) => self.write(&lv, v),
                Update::Mem(mem, a, v) => self.mem_write(mem, &a, v),
            }
        }
    }

    fn exec_block(&mut self, block: &Block, nba: &mut Vec<Update>) -> Flow {
        for stmt in block {
            match self.exec_stmt(stmt, nba) {
                Flow::Next => {}
                other => return other,
            }
        }
        Flow::Next
    }

    fn exec_stmt(&mut self, stmt: &crate::ir::Stmt, nba: &mut Vec<Update>) -> Flow {
        match &stmt.kind {
            StmtKind::Assign {
                target,
                value,
                kind,
                ..
            } => {
                let v = self.value(*value);
                match (kind, target) {
                    (AssignKind::Blocking, _) => self.write(target, v),
                    (AssignKind::NonBlocking, Lvalue::MemElem { mem, addr }) => {
                        let a = self.value(*addr);
                        nba.push(Update::Mem(*mem, a, v));
                    }
                    (AssignKind::NonBlocking, _) => nba.push(Update::Net(target.clone(), v)),
                }
                Flow::Next
            }
            StmtKind::If { cond, then_, else_ } => {
                if self.truth(*cond) == Bit::One {
                    self.exec_block(then_, nba)
                } else {
                    self.exec_block(else_, nba)
                }
            }
            StmtKind::Case {
                subject,
                kind,
                arms,
                default,
                ..
            } => {
                let s = self.value(*subject);
                for arm in arms {
                    for v in &arm.values {
                        let item = self.value(*v);
                        let hit = match kind {
                            CaseKind::Plain => s.case_eq(&item).truth() == Bit::One,
                            CaseKind::Z => s.casez_match(&item),
                            CaseKind::X => s.casex_match(&item),
                        };
                        if hit {
                            return self.exec_block(&arm.body, nba);
                        }
                    }
                }
                match default {
                    Some(d) => self.exec_block(d, nba),
                    None => Flow::Next,
                }
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                if let Some((lv, e)) = init {
                    let v = self.value(*e);
                    self.write(lv, v);
                }
                for _ in 0..10_000 {
                    if let Some(c) = cond
                        && self.truth(*c) != Bit::One
                    {
                        break;
                    }
                    if let Flow::Break = self.exec_block(body, nba) {
                        break;
                    }
                    if let Some((lv, e)) = step {
                        let v = self.value(*e);
                        self.write(lv, v);
                    }
                }
                Flow::Next
            }
            StmtKind::While { cond, body } => {
                for _ in 0..10_000 {
                    if self.truth(*cond) != Bit::One {
                        break;
                    }
                    if let Flow::Break = self.exec_block(body, nba) {
                        break;
                    }
                }
                Flow::Next
            }
            StmtKind::Repeat { count, body } => {
                let n = self.value(*count).to_u64().unwrap_or(0).min(10_000);
                for _ in 0..n {
                    if let Flow::Break = self.exec_block(body, nba) {
                        break;
                    }
                }
                Flow::Next
            }
            StmtKind::Block { body, .. } => self.exec_block(body, nba),
            StmtKind::MemWrite {
                mem,
                addr,
                value,
                enable,
            } => {
                let en = enable.is_none_or(|e| self.truth(e) == Bit::One);
                if en {
                    let a = self.value(*addr);
                    let v = self.value(*value);
                    nba.push(Update::Mem(*mem, a, v));
                }
                Flow::Next
            }
            StmtKind::Break => Flow::Break,
            StmtKind::Continue => Flow::Continue,
            StmtKind::Forever { .. }
            | StmtKind::Wait(_)
            | StmtKind::SysCall { .. }
            | StmtKind::MemFile { .. }
            | StmtKind::Assert { .. }
            | StmtKind::Finish
            | StmtKind::Stop => Flow::Next,
        }
    }

    fn run_process(&mut self, p: &crate::ir::Process) {
        let mut nba = Vec::new();
        self.exec_block(&p.body, &mut nba);
        self.commit(nba);
    }

    fn reset_active(&self, edges: &[Edge]) -> bool {
        edges.iter().any(|e| {
            let v = self.nets[e.net.index()].bit(0);
            match e.polarity {
                Polarity::Pos => v == Bit::One,
                Polarity::Neg => v == Bit::Zero,
                Polarity::Any => false,
            }
        })
    }

    fn rst_active(&mut self, reset: &Reset, rst: ExprId) -> bool {
        let v = self.truth(rst);
        if reset.active_high {
            v == Bit::One
        } else {
            v == Bit::Zero
        }
    }

    /// Evaluates a combinational cell's output.
    fn cell_value(&mut self, cell: &crate::ir::Cell) -> Option<Const> {
        let a = cell.input("a").map(|e| self.value(e));
        let b = cell.input("b").map(|e| self.value(e));
        let y = cell.output("y")?;
        let w = self.nets[y.index()].width();
        Some(match &cell.kind {
            CellKind::Not => a?.not(),
            CellKind::Buf => a?,
            CellKind::And => a?.and(&b?),
            CellKind::Or => a?.or(&b?),
            CellKind::Xor => a?.xor(&b?),
            CellKind::Add => a?.add(&b?),
            CellKind::Sub => a?.sub(&b?),
            CellKind::Mul => a?.mul(&b?),
            CellKind::Div => a?.div(&b?),
            CellKind::Mod => a?.rem(&b?),
            CellKind::Shl => a?.shl_by(&b?),
            CellKind::Shr => a?.shr_by(&b?),
            CellKind::Sshr => a?.sshr_by(&b?),
            CellKind::Eq => a?.eq(&b?),
            CellKind::Ne => a?.ne(&b?),
            CellKind::Lt => a?.lt(&b?),
            CellKind::Le => a?.le(&b?),
            CellKind::Gt => a?.gt(&b?),
            CellKind::Ge => a?.ge(&b?),
            CellKind::ReduceAnd => a?.reduce_and(),
            CellKind::ReduceOr => a?.reduce_or(),
            CellKind::ReduceXor => a?.reduce_xor(),
            CellKind::Mux => {
                let s = self.value(cell.input("s")?);
                match s.truth() {
                    Bit::One => b?,
                    Bit::Zero => a?,
                    _ => crate::synth::eval::merge_unknown(&a?, &b?),
                }
            }
            CellKind::Pmux => {
                let s = self.value(cell.input("s")?);
                let set: Vec<u32> = (0..s.width()).filter(|i| s.bit(*i) == Bit::One).collect();
                match set.as_slice() {
                    [] if s.is_fully_known() => a?,
                    [i] if s.is_fully_known() => b?.slice((i + 1) * w - 1, i * w),
                    _ => Const::x(w),
                }
            }
            CellKind::Lut { init, .. } => match a?.to_u64() {
                Some(i) if i < u64::from(init.width()) => {
                    Const::from_bit(init.bit(u32::try_from(i).ok()?))
                }
                _ => Const::x(1),
            },
            CellKind::Tristate => {
                let en = self.value(cell.input("en")?);
                match en.truth() {
                    Bit::One => a?,
                    Bit::Zero => Const::z(w),
                    _ => Const::x(w),
                }
            }
            _ => return None,
        })
    }

    /// Propagates combinational logic until nothing changes.
    fn settle(&mut self) {
        for _ in 0..64 {
            let before = self.nets.clone();
            let m = self.m;
            for a in &m.assigns {
                let v = self.value(a.value);
                self.write(&a.target, v);
            }
            for (_, p) in m.processes.iter() {
                match &p.kind {
                    ProcessKind::Comb | ProcessKind::Sensitive(_) => self.run_process(p),
                    ProcessKind::Sequential { resets, .. } if self.reset_active(resets) => {
                        self.run_process(p);
                    }
                    _ => {}
                }
            }
            for (_, cell) in m.cells.iter() {
                match &cell.kind {
                    CellKind::Dff { reset: Some(r), .. } if r.asynchronous => {
                        let rst = cell.input("rst").expect("rst");
                        if self.rst_active(r, rst) {
                            let q = cell.output("q").expect("q");
                            self.write(&Lvalue::Net(q), r.value.clone());
                        }
                    }
                    CellKind::Dff { .. } => {}
                    CellKind::Dlatch => {
                        if self.truth(cell.input("en").expect("en")) == Bit::One {
                            let d = self.value(cell.input("d").expect("d"));
                            self.write(&Lvalue::Net(cell.output("q").expect("q")), d);
                        }
                    }
                    CellKind::MemRdPort {
                        mem,
                        clocked: false,
                    } => {
                        let a = self.value(cell.input("addr").expect("addr"));
                        let v = self.mem(*mem, &a).expect("mem");
                        self.write(&Lvalue::Net(cell.output("data").expect("data")), v);
                    }
                    CellKind::MemWrPort {
                        mem,
                        clocked: false,
                    } => {
                        if self.truth(cell.input("en").expect("en")) == Bit::One {
                            let a = self.value(cell.input("addr").expect("addr"));
                            let v = self.value(cell.input("data").expect("data"));
                            self.mem_write(*mem, &a, v);
                        }
                    }
                    CellKind::MemRdPort { .. }
                    | CellKind::MemWrPort { .. }
                    | CellKind::Blackbox(_) => {}
                    _ => {
                        if let Some(v) = self.cell_value(cell) {
                            self.write(&Lvalue::Net(cell.output("y").expect("y")), v);
                        }
                    }
                }
            }
            if self.nets == before {
                return;
            }
        }
    }

    /// A rising edge on `clk`: samples every clocked element from the
    /// current state, then commits and settles.
    fn step(&mut self, clk: NetId) {
        let m = self.m;
        let mut nba: Vec<Update> = Vec::new();
        let mut mem_writes: Vec<Update> = Vec::new();
        for (_, p) in m.processes.iter() {
            if let ProcessKind::Sequential { clocks, .. } = &p.kind
                && clocks
                    .iter()
                    .any(|e| e.net == clk && e.polarity == Polarity::Pos)
            {
                self.exec_block(&p.body, &mut nba);
            }
        }
        for (_, cell) in m.cells.iter() {
            let on_clk = cell
                .input("clk")
                .is_some_and(|e| m.expr(e).as_net() == Some(clk));
            if !on_clk {
                continue;
            }
            match &cell.kind {
                CellKind::Dff {
                    clk_pos: true,
                    has_enable,
                    reset,
                } => {
                    let q = cell.output("q").expect("q");
                    let next = if let Some(r) = reset
                        && self.rst_active(r, cell.input("rst").expect("rst"))
                    {
                        Some(r.value.clone())
                    } else if !*has_enable || self.truth(cell.input("en").expect("en")) == Bit::One
                    {
                        Some(self.value(cell.input("d").expect("d")))
                    } else {
                        None
                    };
                    if let Some(v) = next {
                        nba.push(Update::Net(Lvalue::Net(q), v));
                    }
                }
                CellKind::MemRdPort { mem, clocked: true } => {
                    if self.truth(cell.input("en").expect("en")) == Bit::One {
                        let a = self.value(cell.input("addr").expect("addr"));
                        let v = self.mem(*mem, &a).expect("mem");
                        nba.push(Update::Net(
                            Lvalue::Net(cell.output("data").expect("data")),
                            v,
                        ));
                    }
                }
                CellKind::MemWrPort { mem, clocked: true }
                    if self.truth(cell.input("en").expect("en")) == Bit::One =>
                {
                    let a = self.value(cell.input("addr").expect("addr"));
                    let v = self.value(cell.input("data").expect("data"));
                    mem_writes.push(Update::Mem(*mem, a, v));
                }
                _ => {}
            }
        }
        // Reads (including clocked read ports) were sampled above, so the
        // write ports commit last: read-before-write.
        self.commit(nba);
        self.commit(mem_writes);
        self.settle();
    }
}

/// A small deterministic pseudo-random generator (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn value(&mut self, width: u32) -> Const {
        let mut c = Const::zero(width);
        for i in 0..width {
            if self.next() & 1 == 1 {
                c.set_bit(i, Bit::One);
            }
        }
        c
    }
}

/// True when every known bit of `reference` matches `actual`.
fn compatible(reference: &Const, actual: &Const) -> bool {
    reference.width() == actual.width()
        && (0..reference.width()).all(|i| {
            let r = reference.bit(i);
            !r.is_known() || r == actual.bit(i)
        })
}

/// Drives both modules with the same stimulus and compares their outputs.
fn check_equivalent(name: &str, pre: &Module, post: &Module) {
    let mut a = Sim::new(pre);
    let mut b = Sim::new(post);
    let clocks: Vec<NetId> = pre
        .processes
        .values()
        .filter_map(|p| match &p.kind {
            ProcessKind::Sequential { clocks, .. } => Some(clocks.iter().map(|e| e.net)),
            _ => None,
        })
        .flatten()
        .collect();
    let inputs: Vec<(String, NetId, u32)> = pre
        .ports
        .iter()
        .filter(|p| p.dir == PortDir::In && !clocks.contains(&p.net))
        .map(|p| {
            (
                p.name.to_string(),
                p.net,
                pre.nets[p.net].ty.width().unwrap_or(0),
            )
        })
        .collect();
    let outputs: Vec<(String, NetId)> = pre
        .ports
        .iter()
        .filter(|p| p.dir == PortDir::Out)
        .map(|p| (p.name.to_string(), p.net))
        .collect();
    let post_nets: HashMap<String, NetId> = post
        .ports
        .iter()
        .map(|p| (p.name.to_string(), p.net))
        .collect();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ u64::try_from(name.len()).unwrap_or(0));
    let steps = if clocks.is_empty() { 200 } else { 60 };
    for step in 0..steps {
        for (pname, net, width) in &inputs {
            let mut v = rng.value(*width);
            let is_reset = pname.starts_with("rst") || pname.starts_with("reset");
            if step < 2 && is_reset {
                v = if pname.ends_with("_n") {
                    Const::zero(*width)
                } else {
                    Const::ones(*width)
                };
            }
            a.nets[net.index()] = v.clone();
            b.nets[post_nets[pname].index()] = v;
        }
        a.settle();
        b.settle();
        compare(name, step, "after settle", &a, &b, &outputs, &post_nets);
        for clk in &clocks {
            a.nets[clk.index()] = Const::from_bool(false);
            b.nets[post_nets[&pre.nets[*clk].name.to_string()].index()] = Const::from_bool(false);
            a.settle();
            b.settle();
            a.nets[clk.index()] = Const::from_bool(true);
            b.nets[post_nets[&pre.nets[*clk].name.to_string()].index()] = Const::from_bool(true);
            a.step(*clk);
            b.step(post_nets[&pre.nets[*clk].name.to_string()]);
            compare(name, step, "after clock", &a, &b, &outputs, &post_nets);
        }
    }
}

fn compare(
    name: &str,
    step: usize,
    when: &str,
    a: &Sim<'_>,
    b: &Sim<'_>,
    outputs: &[(String, NetId)],
    post_nets: &HashMap<String, NetId>,
) {
    for (pname, net) in outputs {
        let reference = &a.nets[net.index()];
        let actual = &b.nets[post_nets[pname].index()];
        assert!(
            compatible(reference, actual),
            "{name}: output `{pname}` differs {when} at step {step}: reference {reference}, synthesised {actual}"
        );
    }
}

#[test]
fn golden_designs_behave_identically() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/synth");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy();
            n.ends_with(".rtl") && !n.ends_with(".synth.rtl") && !n.starts_with("unsynth")
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty());
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap();
        let mut map = crate::source::SourceMap::new();
        let file = map.add(name.clone(), text.clone()).unwrap();
        let pre = Design::parse_text(&text, file).unwrap();
        let mut post = pre.clone();
        let mut diags = Diagnostics::new();
        run(&mut post, &SynthOptions::default(), &mut diags);
        assert!(!diags.has_errors(), "{name}: {}", diags.render(&map));
        let top = pre.top.unwrap();
        check_equivalent(&name, pre.module(top), post.module(top));
    }
}

#[test]
fn compatible_treats_unknown_as_dont_care() {
    let x = Const::parse_verilog("4'b1x0z").unwrap();
    assert!(compatible(&x, &Const::from_u64(0b1101, 4)));
    assert!(!compatible(&x, &Const::from_u64(0b0101, 4)));
    assert!(!compatible(&x, &Const::from_u64(0b1101, 5)));
    let mut rng = Rng(1);
    assert_eq!(rng.value(8).width(), 8);
}
