//! Rust co-simulation tests: designs built with `ModuleBuilder` or parsed
//! from `.rtl` text, driven through the `sim::Simulator` API.

#![cfg(feature = "sim")]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use reticle::ir::builder::ModuleBuilder;
use reticle::ir::{Delay, Design, ProcessKind, TimeUnit, Type};
use reticle::logic::Logic;
use reticle::sim::{MemoryFiles, SimOptions, Simulator, Status, Value};
use reticle::source::{SourceMap, Span};

fn span() -> Span {
    let mut map = SourceMap::new();
    let id = map.add("t", "").unwrap();
    Span::new(id, 0, 0)
}

fn l(s: &str) -> Logic {
    Logic::parse_verilog(s).unwrap()
}

fn parse(text: &str) -> Design {
    let mut map = SourceMap::new();
    let file = map.add("t.rtl", text).unwrap();
    match Design::parse_text(text, file) {
        Ok(d) => d,
        Err(diags) => panic!("{}", diags.render(&map)),
    }
}

fn load(text: &str) -> Simulator<'_> {
    // The design must outlive the simulator; leak it for the test's sake.
    let design: &'static Design = Box::leak(Box::new(parse(text)));
    Simulator::new(design, SimOptions::default()).unwrap()
}

fn bits(sim: &Simulator, path: &str) -> Logic {
    sim.get(sim.net(path).unwrap())
}

/// A 32-bit counter with enable and synchronous reset, built with the
/// builder API and clocked from Rust.
fn counter_design() -> Design {
    let span = span();
    let mut b = ModuleBuilder::new("counter", span);
    let clk = b.input("clk", Type::bit());
    let rst = b.input("rst", Type::bit());
    let en = b.input("en", Type::bit());
    let q = b.output_reg("q", Type::bits(32));
    let qv = b.net(q);
    let one = b.const_u64(32, 1);
    let next = b.add(qv, one);
    let zero = b.const_u64(32, 0);
    let rstv = b.net(rst);
    let env = b.net(en);
    let mut p = b.process(Some("count"), ProcessKind::posedge(clk));
    let mut when_rst = b.block();
    when_rst.nonblocking(q, zero);
    let mut when_en = b.block();
    when_en.nonblocking(q, next);
    let mut else_ = b.block();
    else_.if_(env, when_en.finish(), Vec::new());
    p.if_(rstv, when_rst.finish(), else_.finish());
    b.end_process(p);
    let mut design = Design::new();
    let top = design.add_module(b.finish());
    design.top = Some(top);
    design
}

#[test]
fn builder_counter_runs_a_thousand_cycles_quickly() {
    let design = counter_design();
    let mut sim = Simulator::new(&design, SimOptions::default()).unwrap();
    let clk = sim.net("counter.clk").unwrap();
    let rst = sim.net("counter.rst").unwrap();
    let en = sim.net("counter.en").unwrap();
    let q = sim.net("counter.q").unwrap();
    let half = sim.ticks(Delay::new(5, TimeUnit::Ns));
    assert_eq!(half, 5000);
    let edges = Rc::new(RefCell::new(0u32));
    let seen = Rc::clone(&edges);
    sim.on_change(q, move |_, _| *seen.borrow_mut() += 1);
    let start = Instant::now();
    sim.set(clk, l("1'b0"));
    sim.set(rst, l("1'b1"));
    sim.set(en, l("1'b1"));
    for cycle in 0..1000u32 {
        if cycle == 2 {
            sim.set(rst, l("1'b0"));
        }
        sim.set(clk, l("1'b1"));
        sim.run_for(half);
        sim.set(clk, l("1'b0"));
        sim.run_for(half);
    }
    let elapsed = start.elapsed();
    assert_eq!(sim.get(q).to_u64(), Some(998));
    assert_eq!(*edges.borrow(), 999);
    assert_eq!(sim.time(), 1000 * 2 * half);
    assert_eq!(sim.net_name(q), "counter.q");
    assert!(elapsed.as_secs_f64() < 2.0, "1000 cycles took {elapsed:?}");
    assert!(sim.messages().is_empty());
    assert_eq!(sim.status(), Status::Running);
}

#[test]
fn handles_and_listing() {
    let text = "\
top t
module leaf
  net %a u1 wire
  port a in %a
end
module t
  net %x u1 reg
  net %r real reg
  net %i int reg
  memory @m 4 x u8
  instance u0 of leaf (a=%x)
end
";
    let mut sim = load(text);
    assert_eq!(sim.top_name(), "t");
    assert_eq!(sim.precision_fs(), 1000);
    assert!(sim.net("t.x").is_some());
    assert!(sim.net("t.u0.a").is_some());
    assert_eq!(sim.net("t.u0.a"), sim.net("t.x"));
    assert!(sim.net("t.u0.b").is_none());
    assert!(sim.net("nope.x").is_none());
    assert!(sim.net("t.u1.a").is_none());
    assert!(sim.net("x").is_none());
    let names: Vec<String> = sim.nets().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["t.x", "t.r", "t.i", "t.u0.a"]);
    let m = sim.memory("t.m").unwrap();
    assert_eq!(sim.mem_len(m), 4);
    assert!(sim.memory("t.zz").is_none());
    assert!(sim.set_mem(m, 2, l("8'd9")));
    assert!(!sim.set_mem(m, 9, l("8'd9")));
    assert_eq!(sim.get_mem(m, 2), Some(l("8'd9")));
    assert_eq!(sim.get_mem(m, 4), None);
    assert!(format!("{sim:?}").contains("Simulator"));
    let design = sim.design();
    assert_eq!(design.modules.len(), 2);
    let opts = SimOptions {
        top: Some("leaf".into()),
        ..SimOptions::default()
    };
    assert!(format!("{opts:?}").contains("leaf"));
    let s2 = Simulator::new(design, opts).unwrap();
    assert_eq!(s2.top_name(), "leaf");
    let _ = Value::Real(1.0);
}

#[test]
fn force_release_and_stop() {
    let text = "\
top t
module t
  net %a u4 reg
  net %w u4 wire
  net %n u4 reg
  assign %w = add(%a, 4'd1)
  process initial
    %a = 4'd1
    wait for 8'd1
    stop
    %n = 4'd7
    wait for 8'd1
    finish
  end
end
";
    let mut sim = load(text);
    let a = sim.net("t.a").unwrap();
    let w = sim.net("t.w").unwrap();
    let n = sim.net("t.n").unwrap();
    sim.run();
    assert_eq!(sim.status(), Status::Stopped);
    assert_eq!(sim.time(), 1000);
    assert_eq!(sim.get(n), l("4'bxxxx"));
    // Force a wire: it reports the forced value until released, then what
    // its driver resolves to.
    sim.force(w, l("4'd9"));
    assert_eq!(sim.get(w), l("4'd9"));
    sim.set(a, l("4'd3"));
    sim.run_for(0);
    assert_eq!(sim.get(w), l("4'd9"));
    sim.release(w);
    assert_eq!(sim.get(w), l("4'd4"));
    sim.release(w);
    // Force a register: it keeps the forced value after release.
    sim.force(a, l("4'd5"));
    sim.run_for(0);
    assert_eq!(sim.get(w), l("4'd6"));
    sim.release(a);
    assert_eq!(sim.get(a), l("4'd5"));
    sim.run();
    assert!(sim.finished());
    assert_eq!(sim.get(n), l("4'd7"));
    assert_eq!(sim.time(), 2000);
    // Nothing runs after $finish.
    assert!(!sim.step());
    sim.run_for(10);
    assert_eq!(sim.time(), 2000);
}

#[test]
fn step_and_run_until() {
    let text = "\
top t
module t
  net %c u4 reg
  process initial
    %c = 4'd0
    wait for 8'd10
    %c = 4'd1
    wait for 8'd10
    %c = 4'd2
  end
end
";
    let mut sim = load(text);
    let c = sim.net("t.c").unwrap();
    assert!(sim.step());
    assert_eq!(sim.time(), 0);
    assert_eq!(sim.get(c), l("4'd0"));
    assert!(sim.step());
    assert_eq!(sim.time(), 10_000);
    assert_eq!(sim.get(c), l("4'd1"));
    sim.run_until(15_000);
    assert_eq!(sim.time(), 15_000);
    assert_eq!(sim.get(c), l("4'd1"));
    sim.run_until(50_000);
    assert_eq!(sim.time(), 50_000);
    assert_eq!(sim.get(c), l("4'd2"));
    assert!(!sim.step());
    sim.run_for_delay(Delay::new(1, TimeUnit::Us));
    assert_eq!(sim.time(), 1_050_000);
}

#[test]
fn delays_on_assignments_and_drivers() {
    let text = "\
top t
module t
  timescale 1 ns / 1 ns
  net %a u8 reg
  net %b u8 reg
  net %c u8 wire
  net %d u8 reg
  assign %c = %a after 2 ns
  process initial
    %b <= 8'd2 after 3 ns
    wait for 8'd1
    %b <= 8'd4 after 0 ns
    %a = 8'd1 after 5 ns
    %d = %a
  end
end
";
    let mut sim = load(text);
    let a = sim.net("t.a").unwrap();
    let b = sim.net("t.b").unwrap();
    let c = sim.net("t.c").unwrap();
    let d = sim.net("t.d").unwrap();
    sim.run_until(1);
    assert_eq!(sim.get(a), l("8'hxx"));
    assert_eq!(sim.get(b), l("8'd4"));
    sim.run_until(3);
    assert_eq!(sim.get(b), l("8'd2"));
    assert_eq!(sim.get(d), l("8'hxx"));
    // The blocking delayed assignment suspends the process for 5 ns.
    sim.run_until(6);
    assert_eq!(sim.get(a), l("8'd1"));
    assert_eq!(sim.get(d), l("8'd1"));
    assert_eq!(sim.get(c), l("8'hxx"));
    sim.run_until(8);
    assert_eq!(sim.get(c), l("8'd1"));
}

#[test]
fn edges_events_and_loops() {
    let text = "\
top t
module t
  net %clk u1 reg
  net %rst u1 reg
  net %q u4 reg
  net %hits u4 reg
  net %i u4 reg
  net %sum u8 reg
  net %arr [4]u4 reg
  net %pair u8 reg
  process seq negedge %clk async posedge %rst
    if %rst
      %q <= 4'd0
    else
      %q <= add(%q, 4'd1)
    end
  end
  process waiter free
    wait on posedge %clk, negedge %rst
    %hits = add(%hits, 4'd1)
  end
  process loops initial
    %hits = 4'd0
    %sum = 8'd0
    for %i = 4'd0; lt(%i, 4'd10); %i = add(%i, 4'd1)
      if eq(%i, 4'd2)
        continue
      end
      if eq(%i, 4'd5)
        break
      end
      %sum = add(%sum, resize(%i, u8))
    end
    repeat 4'd3
      %sum = add(%sum, 8'd100)
    end
    while lt(%sum, 8'd250)
      %sum = add(%sum, 8'd1)
    end
    forever
      %sum = sub(%sum, 8'd1)
      if eq(%sum, 8'd240)
        break
      end
    end
    %arr[%i] = 4'd9
    %arr[4'd1] = 4'd7
    %arr[4'd9] = 4'd3
    {%pair[7:4], %pair[3:0]} = {4'ha, 4'hb}
  end
  process stim initial
    %clk = 1'd0
    %rst = 1'd1
    wait for 8'd3
    %rst = 1'd0
    wait for 8'd2
    %clk = 1'd1
    wait for 8'd5
    %clk = 1'd0
    wait for 8'd5
    %clk = 1'd1
    wait for 8'd5
    %clk = 1'd0
    wait for 8'd5
  end
end
";
    let mut sim = load(text);
    sim.run();
    assert_eq!(bits(&sim, "t.q"), l("4'd2"));
    // The waiter sees the reset falling edge, then one clock rising edge
    // (it re-arms only after it runs, so alternate edges may be missed).
    assert!(bits(&sim, "t.hits").to_u64().unwrap() >= 2);
    // sum: 0+1+3+4 = 8, +300 = 308 mod 256 = 52, while -> 250, forever -> 240.
    assert_eq!(bits(&sim, "t.sum"), l("8'd240"));
    assert_eq!(bits(&sim, "t.i"), l("4'd5"));
    assert_eq!(bits(&sim, "t.arr"), l("16'hxx7x"));
    assert_eq!(bits(&sim, "t.pair"), l("8'hab"));
    assert!(sim.messages().is_empty(), "{:?}", sim.messages());
}

#[test]
fn runaway_process_and_oscillation_are_reported() {
    let text = "\
top t
module t
  net %x u1 wire
  process spin free
    %x = 1'd0
  end
end
";
    let design = parse(text);
    let opts = SimOptions {
        max_process_steps: 100,
        ..SimOptions::default()
    };
    let mut sim = Simulator::new(&design, opts).unwrap();
    sim.run();
    let msgs = sim.messages();
    assert!(msgs.has_errors());
    assert!(
        msgs.iter()
            .any(|d| d.message.contains("without suspending"))
    );
    let text = "\
top t
module t
  net %p u1 reg
  net %q u1 reg
  process comb
    %p = not(%q)
  end
  process comb
    %q = %p
  end
  process initial
    %p = 1'd0
  end
end
";
    let design = parse(text);
    let opts = SimOptions {
        max_slot_events: 50,
        ..SimOptions::default()
    };
    let mut sim = Simulator::new(&design, opts).unwrap();
    sim.run();
    assert!(sim.finished());
    assert!(
        sim.messages()
            .iter()
            .any(|d| d.message.contains("did not settle"))
    );
}

#[test]
fn reports_and_unknown_tasks() {
    let text = "\
top t
module t
  net %a u1 reg
  process initial
    %a = 1'd0
    assert note %a report \"a is low\"
    assert warning 1'd0 report \"warned %0d\", 8'd5
    assert error 1'd0 report \"errored\"
    sys $info(\"info\")
    sys $warning(\"warn\")
    sys $error(\"err\")
    sys $mystery(%a)
    sys $mystery(%a)
    sys report(\"vhdl report\")
    %a = call $unknown_fn(%a) as u1
    sys $fatal(32'd1, \"fatal %0d\", 8'd7)
    sys $display(\"unreachable\")
  end
end
";
    let mut sim = load(text);
    sim.run();
    assert!(sim.finished());
    assert_eq!(
        sim.output(),
        "Note: a is low\nWarning: warned 5\nError: errored\nNote: info\nWarning: warn\nError: err\nNote: vhdl report\nFailure: fatal 7\n"
    );
    let msgs = sim.take_messages();
    assert_eq!(msgs.error_count(), 3);
    let unknown: Vec<&str> = msgs
        .iter()
        .filter(|d| d.message.contains("unknown system"))
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        unknown,
        [
            "unknown system task `$mystery` is ignored",
            "unknown system function `$unknown_fn` is ignored"
        ]
    );
    assert!(sim.messages().is_empty());
    // A failing assertion of severity `failure` ends the run.
    let text = "\
top t
module t
  process initial
    assert failure 1'd0 report \"boom\"
    sys $display(\"not reached\")
  end
end
";
    let mut sim = load(text);
    sim.run();
    assert!(sim.finished());
    assert_eq!(sim.take_output(), "Failure: boom\n");
    assert_eq!(sim.output(), "");
}

#[test]
fn readmem_through_file_provider() {
    let text = "\
top t
module t
  memory @m 8 x u4
  process initial
    readmemb(\"bits.txt\", @m, 8'd2, 8'd4)
    readmemh(\"missing.hex\", @m)
    readmemh(\"bad.hex\", @m)
    readmemh(4'd0, @m)
    sys $readmemh(\"bits.txt\", @m[4'd0])
    readmemh(\"high.hex\", @m) base 16
  end
end
";
    let design = parse(text);
    let mut files = MemoryFiles::new();
    files
        .insert("bits.txt", "0001 0010 0011 0100 0101")
        .insert("bad.hex", "@zz")
        .insert("high.hex", "@17 e");
    let opts = SimOptions {
        files: Some(Box::new(files)),
        ..SimOptions::default()
    };
    let mut sim = Simulator::new(&design, opts).unwrap();
    sim.run();
    let m = sim.memory("t.m").unwrap();
    assert_eq!(sim.get_mem(m, 1), Some(l("4'hx")));
    assert_eq!(sim.get_mem(m, 2), Some(l("4'd1")));
    assert_eq!(sim.get_mem(m, 4), Some(l("4'd3")));
    assert_eq!(sim.get_mem(m, 5), Some(l("4'hx")));
    // `@17` with element 0 at address 16 is element 1.
    assert_eq!(sim.get_mem(m, 7), Some(l("4'd14")));
    let messages: Vec<String> = sim.messages().iter().map(|d| d.message.clone()).collect();
    assert_eq!(messages.len(), 5, "{messages:?}");
    assert!(messages[0].contains("more words than the 3 element(s) loaded"));
    assert!(messages[1].contains("cannot read `missing.hex`"));
    assert!(messages[2].contains("bad address"));
    assert!(messages[3].contains("file name"));
    assert!(messages[4].contains("names no memory"));
}

#[test]
fn writemem_hands_the_file_back() {
    let text = "\
top t
module t
  memory @m 4 x u4
    init 4'd1 4'd2 4'd3 4'bxz01
  process initial
    writememh(\"all.hex\", @m)
    writememb(\"part.bin\", @m, 2'd2, 2'd1) base 8
    readmemh(\"all.hex\", @m, 2'd0, 2'd0)
  end
end
";
    let design = parse(text);
    let mut sim = Simulator::new(&design, SimOptions::default()).unwrap();
    sim.run();
    let written = sim.written_files();
    assert_eq!(written["all.hex"], "1\n2\n3\nx\n");
    // Downwards from element 2, so every word carries its address, in
    // the file's numbering.
    assert_eq!(written["part.bin"], "@a\n0011\n@9\n0010\n");
    // A saved file reads back in the same run, without a provider; the
    // one-element range makes the rest of it a warning.
    let messages: Vec<String> = sim.messages().iter().map(|d| d.message.clone()).collect();
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(messages[0].contains("more words"), "{messages:?}");
}

#[test]
fn vcd_capture_through_dumpvars_and_writer() {
    let text = "\
top t
module t
  timescale 1 ns / 100 ps
  net %a u1 reg
  net %r real reg
  net %s string reg
  process initial
    sys $dumpfile(\"x.vcd\")
    sys $dumpvars()
    %a = 1'd0
    %r = call $realtime() as real
    wait for 8'd2
    %a = 1'd1
    %r = call $realtime() as real
  end
end
";
    let mut sim = load(text);
    assert!(sim.vcd().is_none());
    sim.run();
    let mut out = String::new();
    sim.dump_vcd(&mut out).unwrap();
    assert_eq!(out, sim.vcd().unwrap());
    let expected = "\
$timescale 100ps $end
$scope module t $end
$var reg 1 ! a $end
$var real 64 \" r $end
$upscope $end
$enddefinitions $end
#0
$dumpvars
x!
rNaN \"
$end
0!
r0 \"
#20
1!
r2 \"
";
    assert_eq!(out, expected);
    // A simulator without capture writes nothing.
    let mut sim = load("top t\nmodule t\nend\n");
    let mut out = String::new();
    sim.dump_vcd(&mut out).unwrap();
    assert!(out.is_empty());
    sim.run();
}

#[test]
fn cells_latch_pmux_lut_and_memory_ports() {
    let text = "\
top t
module t
  net %clk u1 reg
  net %en u1 reg
  net %d u4 reg
  net %q u4 wire
  net %sel u2 reg
  net %opts u8 wire
  net %py u4 wire
  net %lut_in u2 reg
  net %lut_out u1 wire
  net %waddr u2 reg
  net %raddr u2 reg
  net %wdata u8 reg
  net %rdata u8 wire
  net %rdata_async u8 wire
  net %sh u4 wire
  net %buf u4 wire
  net %inv u4 wire
  memory @m 4 x u8
  assign %opts = 8'hab
  cell l0 dlatch (en=%en, d=%d) -> (q=%q)
  cell p0 pmux (a=4'd0, b=%opts, s=%sel) -> (y=%py)
  cell lut0 lut 2 4'b0110 (a=%lut_in) -> (y=%lut_out)
  cell wr0 memwr @m clocked (addr=%waddr, data=%wdata, en=%en, clk=%clk) -> ()
  cell rd0 memrd @m clocked (addr=%raddr, clk=%clk, en=1'd1) -> (data=%rdata)
  cell rd1 memrd @m (addr=%raddr) -> (data=%rdata_async)
  cell sh0 shl (a=%d, b=%sel) -> (y=%sh)
  cell b0 buf (a=%d) -> (y=%buf)
  cell n0 not (a=%d) -> (y=%inv)
  process initial
    %clk = 1'd0
    %en = 1'd1
    %d = 4'd5
    %sel = 2'd1
    %lut_in = 2'd1
    %waddr = 2'd2
    %raddr = 2'd2
    %wdata = 8'd42
    wait for 8'd1
    %en = 1'd0
    %d = 4'd6
    wait for 8'd1
    %clk = 1'd1
    wait for 8'd1
    %clk = 1'd0
    %en = 1'd1
    wait for 8'd1
    %clk = 1'd1
    wait for 8'd1
    %sel = 2'd2
    %lut_in = 2'd3
    wait for 8'd1
    %sel = 2'd3
    wait for 8'd1
  end
end
";
    let mut sim = load(text);
    sim.run_until(1500);
    // The latch holds 5 once en dropped, even though d changed.
    assert_eq!(bits(&sim, "t.q"), l("4'd5"));
    assert_eq!(bits(&sim, "t.py"), l("4'hb"));
    assert_eq!(bits(&sim, "t.lut_out"), l("1'b1"));
    assert_eq!(bits(&sim, "t.sh"), l("4'd12"));
    assert_eq!(bits(&sim, "t.buf"), l("4'd6"));
    assert_eq!(bits(&sim, "t.inv"), l("4'b1001"));
    sim.run_until(2500);
    // The first clock edge had en low: no write, the read port reads x.
    assert_eq!(bits(&sim, "t.rdata"), l("8'hxx"));
    sim.run_until(4500);
    assert_eq!(bits(&sim, "t.rdata_async"), l("8'd42"));
    assert_eq!(bits(&sim, "t.rdata"), l("8'hxx"));
    sim.run_until(5500);
    assert_eq!(bits(&sim, "t.py"), l("4'ha"));
    assert_eq!(bits(&sim, "t.lut_out"), l("1'b0"));
    sim.run();
    assert_eq!(bits(&sim, "t.py"), l("4'hx"));
    assert_eq!(bits(&sim, "t.q"), l("4'd6"));
    assert!(sim.messages().is_empty(), "{:?}", sim.messages());
}

#[test]
fn elaboration_warnings_and_errors() {
    let text = "\
top t
module bb blackbox
  net %o u1 wire
  port o out %o
end
module t
  net %a u1 wire
  net %b u2 wire
  instance u0 of ghost (x=%a)
  instance u1 of bb (o=%a, zz=%a)
  instance u2 of bb (o=%b)
end
";
    let sim = load(text);
    let warnings: Vec<String> = sim.messages().iter().map(|d| d.message.clone()).collect();
    assert_eq!(warnings.len(), 5, "{warnings:?}");
    assert!(warnings[0].contains("unknown module `ghost`"));
    assert!(warnings[1].contains("black box"));
    assert!(warnings[2].contains("does not have"));
    assert!(warnings[4].contains("2 bits"));
    let empty = Design::new();
    assert!(Simulator::new(&empty, SimOptions::default()).is_err());
}
