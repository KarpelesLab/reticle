//! Unit tests for compiled fast mode: eligibility, lowering and running.
//!
//! The heavy correctness work lives in `tests/sim_compiled.rs`, which runs
//! the same design through both engines for thousands of random vectors.
//! What is here is the part that is awkward from outside: the individual
//! reasons a design is refused, and the shapes of program the lowering
//! produces.

use crate::ir::Design;
use crate::logic::Logic;
use crate::source::SourceMap;

use super::{CompileOptions, CompiledSim, Ineligible, Reason, check};

fn parse(text: &str) -> Design {
    let mut map = SourceMap::new();
    let file = map.add("t.rtl", text).unwrap();
    Design::parse_text(text, file).unwrap_or_else(|e| panic!("{e:?}"))
}

fn options() -> CompileOptions {
    CompileOptions {
        zero_init: true,
        ..CompileOptions::default()
    }
}

fn refuse(text: &str) -> Vec<Ineligible> {
    let design = parse(text);
    match check(&design, options()) {
        Ok(_) => panic!("expected the design to be refused"),
        Err(problems) => problems,
    }
}

fn has(problems: &[Ineligible], want: &Reason) -> bool {
    problems.iter().any(|p| &p.reason == want)
}

const COUNTER: &str = "\
module counter
  net %clk u1 wire
  net %rst u1 wire
  net %en u1 wire
  net %q u8 reg
  net %half u4 wire
  port clk in %clk
  port rst in %rst
  port en in %en
  port q out %q
  port half out %half
  assign %half = %q[7:4]
  process count seq posedge %clk
    if %rst
      %q <= 8'd0
    else
      if %en
        %q <= add(%q, 8'd1)
      end
    end
  end
end
";

#[test]
fn a_counter_compiles_and_counts() {
    let design = parse(COUNTER);
    let plan = check(&design, options()).unwrap();
    assert_eq!(plan.top_name(), "counter");
    assert_eq!(plan.clock_rising(), Some(true));
    assert_eq!(plan.state_nets().len(), 1);
    assert!(format!("{plan:?}").contains("counter"));
    let stats = plan.stats();
    assert_eq!(stats.registers, 1);
    assert_eq!(stats.state_bits, 8);
    assert!(stats.comb_ops > 0);
    assert!(!stats.to_string().is_empty());
    let mut sim = plan.compile();
    let rst = sim.net("counter.rst").unwrap();
    let en = sim.net("counter.en").unwrap();
    let q = sim.net("counter.q").unwrap();
    let half = sim.net("counter.half").unwrap();
    assert_eq!(sim.clock(), sim.net("counter.clk"));
    sim.set(rst, Logic::from_bool(true));
    sim.set(en, Logic::from_bool(true));
    sim.step();
    assert_eq!(sim.get(q).to_u64(), Some(0));
    sim.set(rst, Logic::from_bool(false));
    assert_eq!(sim.run_cycles(20), 20);
    assert_eq!(sim.get(q).to_u64(), Some(20));
    assert_eq!(sim.get(half).to_u64(), Some(1));
    assert_eq!(sim.time(), 21);
    // The enable holds the value.
    sim.set(en, Logic::from_bool(false));
    sim.run_cycles(5);
    assert_eq!(sim.get(q).to_u64(), Some(20));
    // Writing a register directly takes effect at the next settle.
    sim.set(q, Logic::from_u64(200, 8));
    assert_eq!(sim.get(q).to_u64(), Some(200));
    assert!(sim.is_settable(q));
    assert!(!sim.is_settable(half));
    assert!(sim.nets().len() >= 5);
    assert_eq!(sim.state_nets(), [("counter.q".to_owned(), q)]);
    assert_eq!(sim.instance_paths(), ["counter"]);
    assert_eq!(sim.net_name(q), "counter.q");
    assert!(format!("{sim:?}").contains("cycle"));
}

#[test]
fn constants_fold_and_subexpressions_are_shared() {
    let design = parse(
        "\
module folded
  net %clk u1 wire
  net %a u8 wire
  net %x u8 wire
  net %y u8 wire
  net %z u8 wire
  net %q u8 reg
  port clk in %clk
  port a in %a
  port q out %q
  assign %x = add(8'd2, 8'd3)
  assign %y = add(%a, %x)
  assign %z = add(%a, %x)
  process seq posedge %clk
    %q <= add(%y, %z)
  end
end
",
    );
    let plan = check(&design, options()).unwrap();
    // `add(2, 3)` folds away and `add(a, x)` is computed once, so the
    // combinational region holds a single add plus the driver copies.
    let adds = plan.stats().comb_ops;
    let mut sim = plan.compile();
    let a = sim.net("folded.a").unwrap();
    let x = sim.net("folded.x").unwrap();
    sim.set(a, Logic::from_u64(10, 8));
    sim.step();
    assert_eq!(sim.get(x).to_u64(), Some(5));
    assert_eq!(sim.get(sim.net("folded.q").unwrap()).to_u64(), Some(30));
    assert!(adds <= 4, "expected sharing, got {adds} ops");
}

#[test]
fn a_free_process_is_refused_by_name() {
    let problems = refuse(
        "\
module tb
  net %clk u1 reg
  process clkgen free
    wait for 8'd5
    %clk = not(%clk)
  end
end
",
    );
    assert!(has(&problems, &Reason::FreeProcess));
    assert!(problems[0].object.contains("clkgen"));
    assert!(problems[0].span.is_some());
    assert!(problems[0].to_string().contains("free-running"));
}

#[test]
fn latches_loops_and_tristate_are_named() {
    let problems = refuse(
        "\
module latch
  net %en u1 wire
  net %d u1 wire
  net %q u1 reg
  port en in %en
  port d in %d
  port q out %q
  process comb
    if %en
      %q = %d
    end
  end
end
",
    );
    assert!(has(&problems, &Reason::Latch));

    let problems = refuse(
        "\
module loop
  net %a u1 wire
  net %b u1 wire
  port a in %a
  assign %b = and(%b, %a)
end
",
    );
    assert!(has(&problems, &Reason::CombinationalLoop));

    let problems = refuse(
        "\
module tri
  net %a u4 wire
  net %en u1 wire
  net %y u4 wire
  port a in %a
  port en in %en
  port y out %y
  cell t0 tristate (a=%a, en=%en) -> (y=%y)
end
",
    );
    assert!(has(&problems, &Reason::Tristate));
}

#[test]
fn two_clocks_and_a_generated_clock_are_refused() {
    let problems = refuse(
        "\
module two
  net %c1 u1 wire
  net %c2 u1 wire
  net %d u1 wire
  net %q1 u1 reg
  net %q2 u1 reg
  port c1 in %c1
  port c2 in %c2
  port d in %d
  process a seq posedge %c1
    %q1 <= %d
  end
  process b seq posedge %c2
    %q2 <= %q1
  end
end
",
    );
    assert!(
        problems
            .iter()
            .any(|p| matches!(p.reason, Reason::SeveralClocks(_)))
    );

    let problems = refuse(
        "\
module gen
  net %c u1 wire
  net %div u1 reg
  net %q u1 reg
  net %d u1 wire
  port c in %c
  port d in %d
  process a seq posedge %c
    %div <= not(%div)
  end
  process b seq posedge %div
    %q <= %d
  end
end
",
    );
    assert!(
        problems
            .iter()
            .any(|p| matches!(p.reason, Reason::SeveralClocks(_) | Reason::GeneratedClock))
    );
}

#[test]
fn unknown_state_needs_zero_init() {
    let design = parse(
        "\
module noreset
  net %clk u1 wire
  net %d u8 wire
  net %q u8 reg
  port clk in %clk
  port d in %d
  port q out %q
  process seq posedge %clk
    %q <= %d
  end
end
",
    );
    let problems = check(&design, CompileOptions::default()).unwrap_err();
    assert!(has(&problems, &Reason::UninitialisedState));
    assert!(check(&design, options()).is_ok());
}

#[test]
fn unknown_constants_and_calls_are_refused() {
    let problems = refuse(
        "\
module unk
  net %clk u1 wire
  net %q u4 reg
  port clk in %clk
  process seq posedge %clk
    %q <= 4'bx01z
  end
end
",
    );
    assert!(
        problems
            .iter()
            .any(|p| matches!(p.reason, Reason::UnknownConstant(_)))
    );

    let problems = refuse(
        "\
module call
  net %clk u1 wire
  net %q u64 reg
  port clk in %clk
  process seq posedge %clk
    %q <= call $time() as u64
  end
end
",
    );
    assert!(
        problems
            .iter()
            .any(|p| matches!(p.reason, Reason::UnsupportedExpression(_)))
    );
}

#[test]
fn timing_control_is_refused() {
    let problems = refuse(
        "\
module timed
  net %clk u1 wire
  net %d u1 wire
  net %q u1 reg
  port clk in %clk
  port d in %d
  process seq posedge %clk
    %q <= %d after 3 ns
  end
end
",
    );
    assert!(has(&problems, &Reason::TimingControl));
}

#[test]
fn a_case_becomes_a_priority_chain() {
    let design = parse(
        "\
module enc
  net %in u4 wire
  net %out u2 wire
  port in in %in
  port out out %out
  process encode comb
    casez %in
      when 4'b1zzz
        %out = 2'd3
      end
      when 4'b01zz
        %out = 2'd2
      end
      when 4'b001z
        %out = 2'd1
      end
      default
        %out = 2'd0
      end
    end
  end
end
",
    );
    let mut sim = check(&design, options()).unwrap().compile();
    let input = sim.net("enc.in").unwrap();
    let out = sim.net("enc.out").unwrap();
    for (value, want) in [(0b1000u64, 3), (0b0100, 2), (0b0011, 1), (0b0001, 0)] {
        sim.set(input, Logic::from_u64(value, 4));
        assert_eq!(sim.get(out).to_u64(), Some(want), "in={value:04b}");
    }
    assert!(sim.clock().is_none());
    // A design with no clock still steps; the step only settles.
    assert!(sim.step());
    assert_eq!(sim.time(), 1);
}

#[test]
fn memories_read_old_and_commit_at_the_edge() {
    let design = parse(
        "\
module ram
  net %clk u1 wire
  net %we u1 wire
  net %addr u4 wire
  net %wdata u8 wire
  net %rdata u8 reg
  net %rom_out u8 wire
  port clk in %clk
  port we in %we
  port addr in %addr
  port wdata in %wdata
  port rdata out %rdata
  port rom_out out %rom_out
  memory @mem 16 x u8
  memory @rom 4 x u8
    init 8'd222 8'd173 8'd190 8'd239
  assign %rom_out = @rom[%addr[1:0]]
  process seq posedge %clk
    memwrite @mem[%addr] = %wdata enable %we
    %rdata <= @mem[%addr]
  end
end
",
    );
    let mut sim = check(&design, options()).unwrap().compile();
    let we = sim.net("ram.we").unwrap();
    let addr = sim.net("ram.addr").unwrap();
    let wdata = sim.net("ram.wdata").unwrap();
    let rdata = sim.net("ram.rdata").unwrap();
    let rom_out = sim.net("ram.rom_out").unwrap();
    let mem = sim.memory("ram.mem").unwrap();
    assert_eq!(sim.mem_len(mem), 16);
    sim.set(we, Logic::from_bool(true));
    sim.set(addr, Logic::from_u64(3, 4));
    sim.set(wdata, Logic::from_u64(77, 8));
    sim.step();
    // The read saw the old contents; the write landed at the commit.
    assert_eq!(sim.get(rdata).to_u64(), Some(0));
    assert_eq!(sim.get_mem(mem, 3).unwrap().to_u64(), Some(77));
    assert_eq!(sim.get(rom_out).to_u64(), Some(239));
    sim.set(we, Logic::from_bool(false));
    sim.step();
    assert_eq!(sim.get(rdata).to_u64(), Some(77));
    assert!(sim.set_mem(mem, 4, Logic::from_u64(9, 8)));
    assert!(!sim.set_mem(mem, 99, Logic::from_u64(9, 8)));
    assert_eq!(sim.get_mem(mem, 4).unwrap().to_u64(), Some(9));
    assert!(sim.get_mem(mem, 99).is_none());
    assert_eq!(sim.memories().len(), 2);
}

#[test]
fn display_and_finish_survive() {
    let design = parse(
        "\
module talky
  net %clk u1 wire
  net %go u1 wire
  net %q u8 reg
  port clk in %clk
  port go in %go
  process seq posedge %clk
    %q <= add(%q, 8'd1)
    if %go
      sys $display(\"q=%0d\", %q)
      finish
    end
  end
end
",
    );
    let mut sim = check(&design, options()).unwrap().compile();
    let go = sim.net("talky.go").unwrap();
    sim.run_cycles(3);
    assert_eq!(sim.output(), "");
    sim.set(go, Logic::from_bool(true));
    sim.step();
    assert_eq!(sim.take_output(), "q=3\n");
    assert!(sim.finished());
    assert_eq!(sim.run_cycles(5), 0);
}

#[test]
fn an_immediate_assertion_reports() {
    let design = parse(
        "\
module checked
  net %clk u1 wire
  net %a u1 wire
  net %q u1 reg
  port clk in %clk
  port a in %a
  process seq posedge %clk
    assert error %a report \"a must hold\"
    %q <= %a
  end
end
",
    );
    let mut sim = check(&design, options()).unwrap().compile();
    let a = sim.net("checked.a").unwrap();
    sim.set(a, Logic::from_bool(true));
    sim.step();
    assert!(sim.messages().is_empty());
    sim.set(a, Logic::from_bool(false));
    sim.step();
    assert_eq!(sim.take_messages().error_count(), 1);
}

#[test]
fn hierarchy_and_initial_blocks_carry_through() {
    let design = parse(
        "\
top top

module adder
  net %a u4 wire
  net %b u4 wire
  net %y u4 wire
  port a in %a
  port b in %b
  port y out %y
  assign %y = add(%a, %b)
end

module top
  net %x u4 reg
  net %y u4 reg
  net %t u4 wire
  net %pair u8 wire
  instance u0 of adder (a=%x, b=%y, y=%t)
  assign %pair[3:0] = %t
  assign %pair[7:4] = %x
  process initial
    %x = 4'd1
    %y = 4'd2
  end
end
",
    );
    let mut sim = check(&design, options()).unwrap().compile();
    assert_eq!(sim.instance_paths(), ["top", "top.u0"]);
    let pair = sim.net("top.pair").unwrap();
    assert_eq!(sim.get(pair).to_u64(), Some(0x13));
    // The child's port aliases the parent's net, so both names work.
    let inner = sim.net("top.u0.y").unwrap();
    assert_eq!(sim.get(inner).to_u64(), Some(3));
    assert_eq!(sim.design().modules.len(), 2);
}

#[test]
fn compiled_sim_new_is_check_plus_compile() {
    let design = parse(COUNTER);
    let sim = CompiledSim::new(&design, options()).unwrap();
    assert_eq!(sim.top_name(), "counter");
    assert_eq!(sim.stats().registers, 1);
    let broken = parse(
        "\
module broken
  net %en u1 wire
  net %d u1 wire
  net %q u1 reg
  port en in %en
  port d in %d
  process comb
    if %en
      %q = %d
    end
  end
end
",
    );
    let err = CompiledSim::new(&broken, options()).unwrap_err();
    assert!(!err.is_empty());
    assert_eq!(super::diagnose(&err).error_count(), err.len());
}

/// Values wider than a machine word exercise the carry-loop kernels and
/// the multi-word slot layout, which no corpus design does.
#[test]
fn values_wider_than_a_word_agree_with_the_event_simulator() {
    let design = parse(
        "\
module wide
  net %clk u1 wire
  net %a u96 wire
  net %b u96 wire
  net %sum u96 wire
  net %prod u96 wire
  net %top u32 wire
  net %acc u96 reg
  port clk in %clk
  port a in %a
  port b in %b
  port sum out %sum
  port prod out %prod
  port top out %top
  port acc out %acc
  assign %sum = add(%a, %b)
  assign %prod = mul(%a, %b)
  assign %top = %sum[95:64]
  process seq posedge %clk
    %acc <= add(%acc, %sum)
  end
end
",
    );
    let mut sim = check(&design, options()).unwrap().compile();
    let a = sim.net("wide.a").unwrap();
    let b = sim.net("wide.b").unwrap();
    // 2^64 - 1 in both operands: the sum carries into the second word and
    // the product fills all three.
    let big = Logic::parse_verilog("96'hffffffffffffffff").unwrap();
    sim.set(a, big.clone());
    sim.set(b, big.clone());
    let sum = sim.get(sim.net("wide.sum").unwrap());
    assert_eq!(sum, big.add(&big));
    let prod = sim.get(sim.net("wide.prod").unwrap());
    assert_eq!(prod, big.mul(&big));
    assert_eq!(
        sim.get(sim.net("wide.top").unwrap()),
        sum.slice(95, 64).resize(32)
    );
    sim.step();
    sim.step();
    let acc = sim.get(sim.net("wide.acc").unwrap());
    assert_eq!(acc, sum.add(&sum));
}

/// Signed operands, where the sign bit decides extension, ordering and
/// the sign of a division.
#[test]
fn signed_operators_keep_their_sign() {
    let design = parse(
        "\
module signs
  net %a s8 wire
  net %b s8 wire
  net %wide s16 wire
  net %quot s8 wire
  net %less u1 wire
  net %shifted s8 wire
  port a in %a
  port b in %b
  port wide out %wide
  port quot out %quot
  port less out %less
  port shifted out %shifted
  assign %wide = resize(%a, s16)
  assign %quot = div(%a, %b)
  assign %less = lt(%a, %b)
  assign %shifted = sshr(%a, 8'd2)
end
",
    );
    let mut sim = check(&design, options()).unwrap().compile();
    let a = sim.net("signs.a").unwrap();
    let b = sim.net("signs.b").unwrap();
    sim.set(a, Logic::from_i64(-7, 8));
    sim.set(b, Logic::from_i64(2, 8));
    assert_eq!(sim.get(sim.net("signs.wide").unwrap()).to_i64(), Some(-7));
    assert_eq!(sim.get(sim.net("signs.quot").unwrap()).to_i64(), Some(-3));
    assert_eq!(sim.get(sim.net("signs.less").unwrap()).to_u64(), Some(1));
    assert_eq!(
        sim.get(sim.net("signs.shifted").unwrap()).to_i64(),
        Some(-2)
    );
    // Division by zero has no two-state answer; fast mode says zero.
    sim.set(b, Logic::zero(8));
    assert_eq!(sim.get(sim.net("signs.quot").unwrap()).to_u64(), Some(0));
}

/// A `for` loop is unrolled, so its trip count has to be static.
#[test]
fn loops_are_unrolled_when_their_bounds_are_static() {
    let design = parse(
        "\
module popcount
  net %in u8 wire
  net %count u4 reg
  net %i u4 var
  port in in %in
  port count out %count
  process sum comb
    %count = 4'd0
    for %i = 4'd0; lt(%i, 4'd8); %i = add(%i, 4'd1)
      %count = add(%count, resize(%in[%i], u4))
    end
  end
end
",
    );
    let mut sim = check(&design, options()).unwrap().compile();
    let input = sim.net("popcount.in").unwrap();
    let count = sim.net("popcount.count").unwrap();
    for v in [0u64, 1, 0xFF, 0b1011_0110] {
        sim.set(input, Logic::from_u64(v, 8));
        let want = u64::from(v.count_ones());
        assert_eq!(sim.get(count).to_u64(), Some(want), "popcount of {v:#b}");
    }

    let problems = refuse(
        "\
module dynamic
  net %n u4 wire
  net %count u4 reg
  net %i u4 var
  port n in %n
  port count out %count
  process sum comb
    %count = 4'd0
    for %i = 4'd0; lt(%i, %n); %i = add(%i, 4'd1)
      %count = add(%count, 4'd1)
    end
  end
end
",
    );
    assert!(has(&problems, &Reason::DynamicLoopBound));
}

/// An asynchronous reset is applied before the settle, so it has to be
/// readable there.
#[test]
fn asynchronous_resets_are_applied_before_the_settle() {
    let design = parse(
        "\
module areset
  net %clk u1 wire
  net %rst_n u1 wire
  net %d u4 wire
  net %q u4 reg
  net %doubled u4 wire
  port clk in %clk
  port rst_n in %rst_n
  port d in %d
  port q out %q
  port doubled out %doubled
  assign %doubled = add(%q, %q)
  process seq posedge %clk async negedge %rst_n
    if lnot(%rst_n)
      %q <= 4'd0
    else
      %q <= %d
    end
  end
end
",
    );
    let mut sim = check(&design, options()).unwrap().compile();
    let rst_n = sim.net("areset.rst_n").unwrap();
    let d = sim.net("areset.d").unwrap();
    let q = sim.net("areset.q").unwrap();
    let doubled = sim.net("areset.doubled").unwrap();
    sim.set(rst_n, Logic::from_bool(true));
    sim.set(d, Logic::from_u64(5, 4));
    sim.step();
    assert_eq!(sim.get(q).to_u64(), Some(5));
    // Asserting the reset clears the register without a clock edge, and
    // the logic it feeds sees that in the same cycle.
    sim.set(rst_n, Logic::from_bool(false));
    assert_eq!(sim.get(q).to_u64(), Some(0));
    assert_eq!(sim.get(doubled).to_u64(), Some(0));

    // A reset that comes out of combinational logic cannot be read that
    // early, so the design is refused rather than mis-simulated.
    let problems = refuse(
        "\
module gated
  net %clk u1 wire
  net %a u1 wire
  net %b u1 wire
  net %rst u1 wire
  net %d u4 wire
  net %q u4 reg
  port clk in %clk
  port a in %a
  port b in %b
  port d in %d
  port q out %q
  assign %rst = and(%a, %b)
  process seq posedge %clk async posedge %rst
    if %rst
      %q <= 4'd0
    else
      %q <= %d
    end
  end
end
",
    );
    assert!(has(&problems, &Reason::AsyncResetLogic));
}

/// Cell-form designs go through the same lowering as the process form.
#[test]
fn the_cell_form_lowers_too() {
    let design = parse(
        "\
module cells
  net %clk u1 wire
  net %rst u1 wire
  net %en u1 wire
  net %q u4 wire
  net %inc u4 wire
  net %q_next u4 wire
  net %is2 u1 wire
  net %parity u1 wire
  net %all_set u1 wire
  net %any_set u1 wire
  port clk in %clk
  port rst in %rst
  port en in %en
  port q out %q
  port is2 out %is2
  port parity out %parity
  port all_set out %all_set
  port any_set out %any_set
  cell add0 add (a=%q, b=4'd1) -> (y=%inc)
  cell mux0 mux (a=%q, b=%inc, s=%en) -> (y=%q_next)
  cell ff0 dff pos arst pos 4'd0 (clk=%clk, d=%q_next, rst=%rst) -> (q=%q)
  cell eq0 eq (a=%q, b=4'd2) -> (y=%is2)
  cell rx0 rxor (a=%q) -> (y=%parity)
  cell ra0 rand (a=%q) -> (y=%all_set)
  cell ro0 ror (a=%q) -> (y=%any_set)
end
",
    );
    let mut sim = check(&design, options()).unwrap().compile();
    let rst = sim.net("cells.rst").unwrap();
    let en = sim.net("cells.en").unwrap();
    let q = sim.net("cells.q").unwrap();
    sim.set(rst, Logic::from_bool(true));
    sim.set(en, Logic::from_bool(true));
    sim.step();
    assert_eq!(sim.get(q).to_u64(), Some(0));
    sim.set(rst, Logic::from_bool(false));
    sim.run_cycles(2);
    assert_eq!(sim.get(q).to_u64(), Some(2));
    assert_eq!(sim.get(sim.net("cells.is2").unwrap()).to_u64(), Some(1));
    assert_eq!(sim.get(sim.net("cells.parity").unwrap()).to_u64(), Some(1));
    // Three reductions of the same net: they share an operand, a width
    // and a signedness, so nothing but the operator distinguishes them
    // and sharing one for another would go unnoticed.
    assert_eq!(sim.get(sim.net("cells.all_set").unwrap()).to_u64(), Some(0));
    assert_eq!(sim.get(sim.net("cells.any_set").unwrap()).to_u64(), Some(1));
    // The asynchronous reset clears the flop between edges.
    sim.set(rst, Logic::from_bool(true));
    assert_eq!(sim.get(q).to_u64(), Some(0));
}

/// `$stop` pauses and the next run resumes, as in the event simulator.
#[test]
fn stop_pauses_and_resumes() {
    let design = parse(
        "\
module pauser
  net %clk u1 wire
  net %go u1 wire
  net %q u4 reg
  port clk in %clk
  port go in %go
  process seq posedge %clk
    %q <= add(%q, 4'd1)
    if %go
      stop
    end
  end
end
",
    );
    let mut sim = check(&design, options()).unwrap().compile();
    let go = sim.net("pauser.go").unwrap();
    let q = sim.net("pauser.q").unwrap();
    sim.set(go, Logic::from_bool(true));
    assert_eq!(sim.run_cycles(5), 1);
    assert_eq!(sim.status(), crate::sim::Status::Stopped);
    assert_eq!(sim.get(q).to_u64(), Some(1));
    sim.set(go, Logic::from_bool(false));
    assert_eq!(sim.run_cycles(3), 3);
    assert_eq!(sim.get(q).to_u64(), Some(4));
}

/// Concatenations, replications and a computed part-select.
#[test]
fn concatenation_targets_are_split_by_width() {
    let design = parse(
        "\
module split
  net %in u12 wire
  net %hi u4 wire
  net %mid u4 wire
  net %lo u4 wire
  net %rep u12 wire
  net %part u4 wire
  net %off u4 wire
  port in in %in
  port off in %off
  port hi out %hi
  port mid out %mid
  port lo out %lo
  port rep out %rep
  port part out %part
  assign {%hi, %mid, %lo} = %in
  assign %rep = {3{%in[3:0]}}
  assign %part = %in[%off +: 4]
end
",
    );
    let mut sim = check(&design, options()).unwrap().compile();
    let input = sim.net("split.in").unwrap();
    let off = sim.net("split.off").unwrap();
    sim.set(input, Logic::from_u64(0xABC, 12));
    sim.set(off, Logic::from_u64(4, 4));
    assert_eq!(sim.get(sim.net("split.hi").unwrap()).to_u64(), Some(0xA));
    assert_eq!(sim.get(sim.net("split.mid").unwrap()).to_u64(), Some(0xB));
    assert_eq!(sim.get(sim.net("split.lo").unwrap()).to_u64(), Some(0xC));
    assert_eq!(sim.get(sim.net("split.rep").unwrap()).to_u64(), Some(0xCCC));
    assert_eq!(sim.get(sim.net("split.part").unwrap()).to_u64(), Some(0xB));
    // An offset past the end reads zero rather than `x`.
    sim.set(off, Logic::from_u64(12, 4));
    assert_eq!(sim.get(sim.net("split.part").unwrap()).to_u64(), Some(0));
}

/// A reset synchroniser: one register's asynchronous reset drives
/// another's. The overrides have to be applied in dependency order, and
/// the process that depends is declared first here so source order would
/// get it wrong.
#[test]
fn a_reset_that_comes_from_a_register_is_applied_first() {
    let design = parse(
        "\
module chained
  net %clk u1 wire
  net %rst_n u1 wire
  net %d u4 wire
  net %sync_n u1 reg
  net %q u4 reg
  port clk in %clk
  port rst_n in %rst_n
  port d in %d
  port sync_n out %sync_n
  port q out %q
  process user seq posedge %clk async negedge %sync_n
    if lnot(%sync_n)
      %q <= 4'd0
    else
      %q <= %d
    end
  end
  process syncer seq posedge %clk async negedge %rst_n
    if lnot(%rst_n)
      %sync_n <= 1'd0
    else
      %sync_n <= 1'd1
    end
  end
end
",
    );
    let mut sim = check(&design, options()).unwrap().compile();
    let rst_n = sim.net("chained.rst_n").unwrap();
    let d = sim.net("chained.d").unwrap();
    let q = sim.net("chained.q").unwrap();
    let sync_n = sim.net("chained.sync_n").unwrap();
    sim.set(rst_n, Logic::from_bool(true));
    sim.set(d, Logic::from_u64(9, 4));
    sim.run_cycles(3);
    assert_eq!(sim.get(sync_n).to_u64(), Some(1));
    assert_eq!(sim.get(q).to_u64(), Some(9));
    // Both registers clear in the same settle, without a clock edge.
    sim.set(rst_n, Logic::from_bool(false));
    assert_eq!(sim.get(sync_n).to_u64(), Some(0));
    assert_eq!(sim.get(q).to_u64(), Some(0));
}

/// Two asynchronous resets that reset each other have no order to be
/// applied in.
#[test]
fn resets_that_depend_on_each_other_are_refused() {
    let problems = refuse(
        "\
module knot
  net %clk u1 wire
  net %a u1 reg
  net %b u1 reg
  port clk in %clk
  port a out %a
  port b out %b
  process pa seq posedge %clk async negedge %b
    if lnot(%b)
      %a <= 1'd0
    else
      %a <= 1'd1
    end
  end
  process pb seq posedge %clk async negedge %a
    if lnot(%a)
      %b <= 1'd0
    else
      %b <= 1'd1
    end
  end
end
",
    );
    assert!(has(&problems, &Reason::AsyncResetLogic));
}
