//! Interconnect generators: parameterised IP written in Rust.
//!
//! This is the roadmap's "generators" idea on a real case. A 4×4
//! AXI4-Lite crossbar is about six hundred lines of Verilog, nearly all
//! of it index arithmetic that a `generate` loop expresses badly and a
//! reviewer cannot check. Written against [`ModuleBuilder`] it is a
//! function of `N`, `M` and an address map, and what comes out is an
//! [`ir::Module`] that every later stage — validation, simulation,
//! synthesis, emission — treats exactly like a module that was parsed
//! from source.
//!
//! [`ir::Module`]: crate::ir::Module
//!
//! Two generators live here:
//!
//! - [`Crossbar`]: an AXI4-Lite crossbar, N managers by M subordinates,
//!   with an address decode map and round-robin arbitration.
//! - [`WishboneArbiter`]: a Wishbone shared-bus arbiter, N managers onto
//!   one bus, also round-robin.
//!
//! Both build their ports from the bus definitions in [`super::bus`], so
//! a generated crossbar is recognised by [`super::bus::match_ports`] by
//! construction rather than by agreement between two pieces of code.
//!
//! # What the crossbar does, exactly
//!
//! It is a **shared-access crossbar**: full N×M connectivity, one
//! transaction in flight. An arbiter grants one manager at a time, the
//! granted manager's address is decoded to one subordinate, and every
//! channel is routed between those two until the transaction's last
//! handshake. The next grant starts at the manager after the last one,
//! so no manager can be starved.
//!
//! That is deliberately the simple thing rather than the fast thing. A
//! full crossbar with a per-subordinate arbiter, outstanding
//! transactions and reordering is a much larger machine, and a generator
//! whose output cannot be read is a generator whose output cannot be
//! trusted. The structure here — decode, arbitrate, route — is the one a
//! faster version would keep.
//!
//! An address that matches no subordinate is answered by the crossbar
//! itself with `DECERR` (`resp = 2'b11`), which is what the protocol
//! asks for and what keeps a mistyped address from hanging the bus.
//!
//! Arbitration is on the **address channel**: a manager asserting
//! `wvalid` without `awvalid` is not yet asking for anything. AXI4-Lite
//! permits write data to lead the address, so such a manager waits until
//! it presents the address.
//!
//! # Testing
//!
//! The generators are tested by *running* them: the unit tests below
//! build a crossbar, wire real subordinates to it and drive AXI
//! transactions through the simulator, checking the data that comes back
//! and the `DECERR` that an unmapped address produces. Inspecting the
//! netlist would only prove that the generator did what it was written
//! to do.

use std::collections::BTreeMap;

use super::bus::{self, BusInterface, BusRole};
use crate::ir::builder::ModuleBuilder;
use crate::ir::{Edge, ExprId, Module, NetId, PortDir, ProcessKind, Type};
use crate::source::Span;

/// The `resp` code for an address that decodes to nothing.
const DECERR: u64 = 0b11;

/// State encoding of the crossbar's arbiter.
const STATE_IDLE: u64 = 0;
/// The granted manager is doing a write.
const STATE_WRITE: u64 = 1;
/// The granted manager is doing a read.
const STATE_READ: u64 = 2;

/// Number of bits needed to index `n` things, never fewer than one.
fn idx_width(n: usize) -> u32 {
    if n <= 1 {
        return 1;
    }
    let last = u64::try_from(n - 1).expect("a count fits in u64");
    64 - last.leading_zeros()
}

/// An index as a constant value, without a lossy cast.
fn idx(value: usize) -> u64 {
    u64::try_from(value).expect("an index fits in u64")
}

// ---------------------------------------------------------------------------
// Address maps
// ---------------------------------------------------------------------------

/// One subordinate's slice of the address space.
///
/// Decoding is by mask, as every real crossbar does it, so `size` is
/// rounded **up** to a power of two by [`AddressRange::new`] and `base`
/// should be a multiple of that size. [`Crossbar::problems`] reports a
/// base that is not, and an overlap between two ranges, rather than
/// silently generating a decoder that answers the wrong subordinate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AddressRange {
    /// The first address in the range.
    pub base: u64,
    /// The size in bytes, always a power of two.
    pub size: u64,
}

impl AddressRange {
    /// A range at `base`, at least `size` bytes, rounded up to a power
    /// of two.
    ///
    /// ```
    /// use reticle::ip::AddressRange;
    /// assert_eq!(AddressRange::new(0x1000, 0x800).size, 0x800);
    /// assert_eq!(AddressRange::new(0x1000, 0x900).size, 0x1000);
    /// assert_eq!(AddressRange::new(0, 0).size, 1);
    /// ```
    pub fn new(base: u64, size: u64) -> Self {
        let size = size.max(1).next_power_of_two();
        AddressRange { base, size }
    }

    /// The mask a decoder compares under: the bits above the range.
    pub fn mask(self) -> u64 {
        !(self.size - 1)
    }

    /// The first address past the range.
    pub fn end(self) -> u64 {
        self.base.saturating_add(self.size)
    }

    /// True when `address` decodes to this range.
    pub fn contains(self, address: u64) -> bool {
        address & self.mask() == self.base & self.mask()
    }

    /// True when `base` is a multiple of `size`, which is what makes
    /// mask decoding exact.
    pub fn is_aligned(self) -> bool {
        self.base & (self.size - 1) == 0
    }
}

// ---------------------------------------------------------------------------
// The AXI4-Lite crossbar
// ---------------------------------------------------------------------------

/// A generated AXI4-Lite crossbar.
///
/// The module it builds has, besides `clk` and `rst_n`:
///
/// - a full AXI4-Lite **subordinate** interface `s<i>_` per manager,
/// - a full AXI4-Lite **manager** interface `m<j>_` per subordinate.
///
/// ```
/// use reticle::ip::{AddressRange, Crossbar};
/// use reticle::ir::validate::validate;
/// use reticle::source::{SourceMap, Span};
///
/// let mut map = SourceMap::new();
/// let span = Span::new(map.add("generated", "").unwrap(), 0, 0);
/// let module = Crossbar::new(
///     "axil_xbar",
///     2,
///     vec![AddressRange::new(0x0000, 0x1000), AddressRange::new(0x1000, 0x1000)],
///     span,
/// )
/// .build();
/// assert_eq!(module.name, "axil_xbar");
/// assert!(module.port("s0_awvalid").is_some());
/// assert!(module.port("m1_rdata").is_some());
/// assert!(validate(&{
///     let mut d = reticle::ir::Design::new();
///     d.top = Some(d.add_module(module));
///     d
/// })
/// .is_empty());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crossbar {
    /// The generated module's name.
    pub name: String,
    /// How many managers attach to it.
    pub managers: usize,
    /// The subordinates, with the address range each answers.
    pub subordinates: Vec<AddressRange>,
    /// The address width of every interface.
    pub addr_width: u32,
    /// The data width of every interface.
    pub data_width: u32,
    /// The span given to every generated object.
    pub span: Span,
}

impl Crossbar {
    /// A crossbar with the default 32-bit address and data widths.
    pub fn new(
        name: impl Into<String>,
        managers: usize,
        subordinates: Vec<AddressRange>,
        span: Span,
    ) -> Self {
        Crossbar {
            name: name.into(),
            managers,
            subordinates,
            addr_width: 32,
            data_width: 32,
            span,
        }
    }

    /// The same crossbar with other widths.
    pub fn with_widths(mut self, addr_width: u32, data_width: u32) -> Self {
        self.addr_width = addr_width;
        self.data_width = data_width;
        self
    }

    /// The byte-strobe width, never zero.
    fn strobe_width(&self) -> u32 {
        (self.data_width / 8).max(1)
    }

    /// Everything wrong with the configuration, in words.
    ///
    /// An empty result means [`build`](Crossbar::build) will produce a
    /// decoder that is exact. The problems are returned rather than
    /// reported so a caller can turn them into diagnostics against
    /// whatever asked for the crossbar.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.managers == 0 {
            out.push("a crossbar with no managers can do nothing".to_owned());
        }
        if self.subordinates.is_empty() {
            out.push("a crossbar with no subordinates answers DECERR to everything".to_owned());
        }
        if self.addr_width == 0 || self.addr_width > 64 {
            out.push(format!(
                "the address width {} is outside 1..=64",
                self.addr_width
            ));
        }
        if self.data_width == 0 || !self.data_width.is_multiple_of(8) {
            out.push(format!(
                "the data width {} is not a whole number of bytes",
                self.data_width
            ));
        }
        for (i, range) in self.subordinates.iter().enumerate() {
            if !range.is_aligned() {
                out.push(format!(
                    "subordinate {i} is {} bytes at {:#x}, which is not a multiple of its size",
                    range.size, range.base
                ));
            }
            if self.addr_width < 64 && range.end() > 1u64 << self.addr_width {
                out.push(format!(
                    "subordinate {i} ends at {:#x}, past the {}-bit address space",
                    range.end(),
                    self.addr_width
                ));
            }
            for (j, other) in self.subordinates.iter().enumerate().skip(i + 1) {
                if range.base < other.end() && other.base < range.end() {
                    out.push(format!(
                        "subordinates {i} and {j} overlap at {:#x}",
                        range.base.max(other.base)
                    ));
                }
            }
        }
        out
    }

    /// Builds the crossbar as an IR module.
    #[allow(clippy::too_many_lines)]
    pub fn build(&self) -> Module {
        let axi = bus::builtin("axi4lite").expect("axi4lite is a built-in bus");
        let bindings = BTreeMap::from([
            ("ADDR_WIDTH".to_owned(), self.addr_width),
            ("DATA_WIDTH".to_owned(), self.data_width),
        ]);
        let n = self.managers;
        let m = self.subordinates.len();
        let grant_w = idx_width(n);
        let sel_w = idx_width(m + 1);
        let a = self.addr_width;
        let d = self.data_width;
        let s = self.strobe_width();

        let mut b = ModuleBuilder::new(self.name.clone(), self.span);
        b.attr("generator", "reticle::ip::interconnect::Crossbar");
        b.param("ADDR_WIDTH", i64::from(a));
        b.param("DATA_WIDTH", i64::from(d));
        b.param("MANAGERS", i64::try_from(n).unwrap_or(i64::MAX));
        b.param("SUBORDINATES", i64::try_from(m).unwrap_or(i64::MAX));

        let clk = b.input("clk", Type::bit());
        let rst_n = b.input("rst_n", Type::bit());
        let mgr = add_bus_ports(
            &mut b,
            axi,
            BusRole::Subordinate,
            &prefixes("s", n),
            &bindings,
        );
        let sub = add_bus_ports(&mut b, axi, BusRole::Manager, &prefixes("m", m), &bindings);

        // --- arbitration state ------------------------------------------
        let state = b.add_reg("state", Type::bits(2));
        let grant = b.add_reg("grant", Type::bits(grant_w));
        let sel = b.add_reg("sel", Type::bits(sel_w));
        let rr = b.add_reg("rr", Type::bits(grant_w));
        let addr_done = b.add_reg("addr_done", Type::bit());
        let w_done = b.add_reg("w_done", Type::bit());

        // --- per-manager requests ---------------------------------------
        let mut reqs = Vec::with_capacity(n);
        for (i, ports) in mgr.iter().enumerate() {
            let net = b.add_net(format!("req{i}"), Type::bit());
            let aw = b.net(ports["awvalid"]);
            let ar = b.net(ports["arvalid"]);
            let value = b.or(aw, ar);
            b.assign(net, value);
            reqs.push(net);
        }
        let any_req = b.add_net("any_req", Type::bit());
        let value = reduce_or(&mut b, &reqs);
        b.assign(any_req, value);

        // --- round robin: the first requester after `rr` ------------------
        let nxt_grant = b.add_net("nxt_grant", Type::bits(grant_w));
        let value = round_robin(&mut b, rr, &reqs, grant_w);
        b.assign(nxt_grant, value);

        // What the next grant is asking for, which the FSM latches.
        let nxt_is_write = b.add_net("nxt_is_write", Type::bit());
        let arms: Vec<ExprId> = mgr.iter().map(|p| b.net(p["awvalid"])).collect();
        let default = b.const_bit(false);
        let value = select(&mut b, nxt_grant, grant_w, &arms, default);
        b.assign(nxt_is_write, value);

        let nxt_addr = b.add_net("nxt_addr", Type::bits(a));
        let mut arms = Vec::with_capacity(n);
        for ports in &mgr {
            let awvalid = b.net(ports["awvalid"]);
            let awaddr = b.net(ports["awaddr"]);
            let araddr = b.net(ports["araddr"]);
            arms.push(b.mux(awvalid, awaddr, araddr));
        }
        let default = b.const_u64(a, 0);
        let value = select(&mut b, nxt_grant, grant_w, &arms, default);
        b.assign(nxt_addr, value);

        // --- address decode ----------------------------------------------
        let nxt_sel = b.add_net("nxt_sel", Type::bits(sel_w));
        let mut value = b.const_u64(sel_w, idx(m));
        for (j, range) in self.subordinates.iter().enumerate().rev() {
            let addr = b.net(nxt_addr);
            let mask = b.const_u64(a, range.mask());
            let masked = b.and(addr, mask);
            let base = b.const_u64(a, range.base & range.mask());
            let hit = b.eq(masked, base);
            let this = b.const_u64(sel_w, idx(j));
            value = b.mux(hit, this, value);
        }
        b.assign(nxt_sel, value);

        // --- state decode -------------------------------------------------
        let in_write = b.add_net("in_write", Type::bit());
        let sv = b.net(state);
        let k = b.const_u64(2, STATE_WRITE);
        let value = b.eq(sv, k);
        b.assign(in_write, value);
        let in_read = b.add_net("in_read", Type::bit());
        let sv = b.net(state);
        let k = b.const_u64(2, STATE_READ);
        let value = b.eq(sv, k);
        b.assign(in_read, value);

        let mut grant_is = Vec::with_capacity(n);
        for i in 0..n {
            let net = b.add_net(format!("grant_is{i}"), Type::bit());
            let g = b.net(grant);
            let k = b.const_u64(grant_w, idx(i));
            let value = b.eq(g, k);
            b.assign(net, value);
            grant_is.push(net);
        }
        let mut sel_is = Vec::with_capacity(m);
        for j in 0..m {
            let net = b.add_net(format!("sel_is{j}"), Type::bit());
            let sv = b.net(sel);
            let k = b.const_u64(sel_w, idx(j));
            let value = b.eq(sv, k);
            b.assign(net, value);
            sel_is.push(net);
        }
        // `sel == m` is the index no subordinate claimed; it needs no
        // net of its own, because it is what every `select` below falls
        // through to.

        // --- the granted manager's side of every channel -------------------
        let forward: [(&str, u32); 9] = [
            ("awaddr", a),
            ("awprot", 3),
            ("awvalid", 1),
            ("wdata", d),
            ("wstrb", s),
            ("wvalid", 1),
            ("bready", 1),
            ("araddr", a),
            ("arprot", 3),
        ];
        let mut from_manager = BTreeMap::new();
        for (signal, width) in forward.into_iter().chain([("arvalid", 1), ("rready", 1)]) {
            let net = b.add_net(format!("mgr_{signal}"), Type::bits(width));
            let arms: Vec<ExprId> = mgr.iter().map(|p| b.net(p[signal])).collect();
            let default = b.const_u64(width, 0);
            let value = select(&mut b, grant, grant_w, &arms, default);
            b.assign(net, value);
            from_manager.insert(signal, net);
        }

        // --- the selected subordinate's side, with the error responder ----
        let not_addr_done = b.add_net("no_addr_yet", Type::bit());
        let v = b.net(addr_done);
        let value = b.lnot(v);
        b.assign(not_addr_done, value);
        let not_w_done = b.add_net("no_data_yet", Type::bit());
        let v = b.net(w_done);
        let value = b.lnot(v);
        b.assign(not_w_done, value);
        let both_done = b.add_net("write_complete", Type::bit());
        let a1 = b.net(addr_done);
        let a2 = b.net(w_done);
        let value = b.and(a1, a2);
        b.assign(both_done, value);

        let decerr = b.const_u64(2, DECERR);
        let zero_data = b.const_u64(d, 0);
        let error_arm: BTreeMap<&str, ExprId> = BTreeMap::from([
            ("awready", b.net(not_addr_done)),
            ("wready", b.net(not_w_done)),
            ("bvalid", b.net(both_done)),
            ("bresp", decerr),
            ("arready", b.net(not_addr_done)),
            ("rvalid", b.net(addr_done)),
            ("rdata", zero_data),
            ("rresp", decerr),
        ]);

        let backward: [(&str, u32); 8] = [
            ("awready", 1),
            ("wready", 1),
            ("bvalid", 1),
            ("bresp", 2),
            ("arready", 1),
            ("rvalid", 1),
            ("rdata", d),
            ("rresp", 2),
        ];
        let mut from_subordinate = BTreeMap::new();
        for (signal, width) in backward {
            let net = b.add_net(format!("sub_{signal}"), Type::bits(width));
            let arms: Vec<ExprId> = sub.iter().map(|p| b.net(p[signal])).collect();
            let default = error_arm[signal];
            let value = select(&mut b, sel, sel_w, &arms, default);
            b.assign(net, value);
            from_subordinate.insert(signal, net);
        }

        // --- drive the subordinates ---------------------------------------
        for (j, ports) in sub.iter().enumerate() {
            let gated: [(&str, NetId, NetId); 5] = [
                ("awvalid", from_manager["awvalid"], in_write),
                ("wvalid", from_manager["wvalid"], in_write),
                ("bready", from_manager["bready"], in_write),
                ("arvalid", from_manager["arvalid"], in_read),
                ("rready", from_manager["rready"], in_read),
            ];
            for (signal, source, phase) in gated {
                let phase = b.net(phase);
                let chosen = b.net(sel_is[j]);
                let both = b.and(phase, chosen);
                let value = b.net(source);
                let value = b.and(both, value);
                b.assign(ports[signal], value);
            }
            for signal in ["awaddr", "awprot", "wdata", "wstrb", "araddr", "arprot"] {
                let value = b.net(from_manager[signal]);
                b.assign(ports[signal], value);
            }
        }

        // --- drive the managers -------------------------------------------
        for (i, ports) in mgr.iter().enumerate() {
            let gated: [(&str, &str, NetId); 5] = [
                ("awready", "awready", in_write),
                ("wready", "wready", in_write),
                ("bvalid", "bvalid", in_write),
                ("arready", "arready", in_read),
                ("rvalid", "rvalid", in_read),
            ];
            for (port, signal, phase) in gated {
                let phase = b.net(phase);
                let mine = b.net(grant_is[i]);
                let both = b.and(phase, mine);
                let value = b.net(from_subordinate[signal]);
                let value = b.and(both, value);
                b.assign(ports[port], value);
            }
            for signal in ["bresp", "rdata", "rresp"] {
                let value = b.net(from_subordinate[signal]);
                b.assign(ports[signal], value);
            }
        }

        // --- the arbiter ---------------------------------------------------
        let zero_state = b.const_u64(2, STATE_IDLE);
        let zero_grant = b.const_u64(grant_w, 0);
        let zero_sel = b.const_u64(sel_w, 0);
        let false_ = b.const_bit(false);
        let true_ = b.const_bit(true);
        let write_state = b.const_u64(2, STATE_WRITE);
        let read_state = b.const_u64(2, STATE_READ);

        let mut reset = b.block();
        reset.nonblocking(state, zero_state);
        reset.nonblocking(grant, zero_grant);
        reset.nonblocking(sel, zero_sel);
        reset.nonblocking(rr, zero_grant);
        reset.nonblocking(addr_done, false_);
        reset.nonblocking(w_done, false_);

        // IDLE: take the next requester, latch what it wants.
        let mut start = b.block();
        let g = b.net(nxt_grant);
        start.nonblocking(grant, g);
        let g = b.net(nxt_grant);
        start.nonblocking(rr, g);
        let sl = b.net(nxt_sel);
        start.nonblocking(sel, sl);
        let is_write = b.net(nxt_is_write);
        let next_state = b.mux(is_write, write_state, read_state);
        start.nonblocking(state, next_state);
        start.nonblocking(addr_done, false_);
        start.nonblocking(w_done, false_);
        let mut idle = b.block();
        let cond = b.net(any_req);
        idle.if_(cond, start.finish(), Vec::new());

        // WRITE: track the two request handshakes, end on the response.
        let mut write = b.block();
        let ready = b.net(from_subordinate["awready"]);
        let valid = b.net(from_manager["awvalid"]);
        let shook = b.and(ready, valid);
        let mut set = b.block();
        set.nonblocking(addr_done, true_);
        write.if_(shook, set.finish(), Vec::new());
        let ready = b.net(from_subordinate["wready"]);
        let valid = b.net(from_manager["wvalid"]);
        let shook = b.and(ready, valid);
        let mut set = b.block();
        set.nonblocking(w_done, true_);
        write.if_(shook, set.finish(), Vec::new());
        let valid = b.net(from_subordinate["bvalid"]);
        let ready = b.net(from_manager["bready"]);
        let shook = b.and(valid, ready);
        let mut done = b.block();
        done.nonblocking(state, zero_state);
        write.if_(shook, done.finish(), Vec::new());

        // READ: the address handshake, then the data handshake.
        let mut read = b.block();
        let ready = b.net(from_subordinate["arready"]);
        let valid = b.net(from_manager["arvalid"]);
        let shook = b.and(ready, valid);
        let mut set = b.block();
        set.nonblocking(addr_done, true_);
        read.if_(shook, set.finish(), Vec::new());
        let valid = b.net(from_subordinate["rvalid"]);
        let ready = b.net(from_manager["rready"]);
        let shook = b.and(valid, ready);
        let mut done = b.block();
        done.nonblocking(state, zero_state);
        read.if_(shook, done.finish(), Vec::new());

        let mut running = b.block();
        let is_write = b.net(in_write);
        let mut not_idle = b.block();
        not_idle.if_(is_write, write.finish(), read.finish());
        let sv = b.net(state);
        let k = b.const_u64(2, STATE_IDLE);
        let is_idle = b.eq(sv, k);
        running.if_(is_idle, idle.finish(), not_idle.finish());

        let mut process = b.process(
            Some("arbiter"),
            ProcessKind::Sequential {
                clocks: vec![Edge::pos(clk)],
                resets: vec![Edge::neg(rst_n)],
            },
        );
        let r = b.net(rst_n);
        let in_reset = b.lnot(r);
        process.if_(in_reset, reset.finish(), running.finish());
        b.end_process(process);

        b.finish()
    }
}

// ---------------------------------------------------------------------------
// The Wishbone arbiter
// ---------------------------------------------------------------------------

/// A generated Wishbone shared-bus arbiter.
///
/// N managers attach as `s<i>_`; the shared bus leaves as `m_`. A
/// manager is granted the bus while it holds `cyc`, so a read-modify-write
/// cycle is not broken up, and the next grant starts at the manager after
/// the last one.
///
/// Wishbone has no address decode of its own: every subordinate sees the
/// whole shared bus and picks its own transactions out of it, which is
/// what makes an arbiter the right shape here and a crossbar the right
/// shape for AXI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WishboneArbiter {
    /// The generated module's name.
    pub name: String,
    /// How many managers share the bus.
    pub managers: usize,
    /// The address width.
    pub addr_width: u32,
    /// The data width.
    pub data_width: u32,
    /// The span given to every generated object.
    pub span: Span,
}

impl WishboneArbiter {
    /// An arbiter with the default 32-bit address and data widths.
    pub fn new(name: impl Into<String>, managers: usize, span: Span) -> Self {
        WishboneArbiter {
            name: name.into(),
            managers,
            addr_width: 32,
            data_width: 32,
            span,
        }
    }

    /// The same arbiter with other widths.
    pub fn with_widths(mut self, addr_width: u32, data_width: u32) -> Self {
        self.addr_width = addr_width;
        self.data_width = data_width;
        self
    }

    /// Builds the arbiter as an IR module.
    pub fn build(&self) -> Module {
        let wb = bus::builtin("wishbone").expect("wishbone is a built-in bus");
        let bindings = BTreeMap::from([
            ("ADDR_WIDTH".to_owned(), self.addr_width),
            ("DATA_WIDTH".to_owned(), self.data_width),
        ]);
        let n = self.managers;
        let grant_w = idx_width(n);

        let mut b = ModuleBuilder::new(self.name.clone(), self.span);
        b.attr("generator", "reticle::ip::interconnect::WishboneArbiter");
        b.param("ADDR_WIDTH", i64::from(self.addr_width));
        b.param("DATA_WIDTH", i64::from(self.data_width));
        b.param("MANAGERS", i64::try_from(n).unwrap_or(i64::MAX));

        let clk = b.input("clk", Type::bit());
        let rst_n = b.input("rst_n", Type::bit());
        let mgr = add_bus_ports(
            &mut b,
            wb,
            BusRole::Subordinate,
            &prefixes("s", n),
            &bindings,
        );
        let shared = add_bus_ports(
            &mut b,
            wb,
            BusRole::Manager,
            std::slice::from_ref(&"m_".to_owned()),
            &bindings,
        );
        let shared = &shared[0];

        let grant = b.add_reg("grant", Type::bits(grant_w));
        let rr = b.add_reg("rr", Type::bits(grant_w));
        let busy = b.add_reg("busy", Type::bit());

        let reqs: Vec<NetId> = (0..n)
            .map(|i| {
                let net = b.add_net(format!("req{i}"), Type::bit());
                let value = b.net(mgr[i]["cyc"]);
                b.assign(net, value);
                net
            })
            .collect();
        let any_req = b.add_net("any_req", Type::bit());
        let value = reduce_or(&mut b, &reqs);
        b.assign(any_req, value);

        let nxt_grant = b.add_net("nxt_grant", Type::bits(grant_w));
        let value = round_robin(&mut b, rr, &reqs, grant_w);
        b.assign(nxt_grant, value);

        let mut grant_is = Vec::with_capacity(n);
        for i in 0..n {
            let net = b.add_net(format!("grant_is{i}"), Type::bit());
            let g = b.net(grant);
            let k = b.const_u64(grant_w, idx(i));
            let value = b.eq(g, k);
            b.assign(net, value);
            grant_is.push(net);
        }

        // The shared bus carries the granted manager's request, gated by
        // `busy` so nothing is driven between cycles.
        let strobes = ["cyc", "stb"];
        for signal in strobes {
            let arms: Vec<ExprId> = mgr.iter().map(|p| b.net(p[signal])).collect();
            let default = b.const_bit(false);
            let value = select(&mut b, grant, grant_w, &arms, default);
            let gate = b.net(busy);
            let value = b.and(gate, value);
            b.assign(shared[signal], value);
        }
        for signal in ["adr", "dat_w", "sel", "we"] {
            let width = wb
                .signal(signal)
                .and_then(|s| s.width.resolve(&bindings))
                .unwrap_or(1);
            let arms: Vec<ExprId> = mgr.iter().map(|p| b.net(p[signal])).collect();
            let default = b.const_u64(width, 0);
            let value = select(&mut b, grant, grant_w, &arms, default);
            b.assign(shared[signal], value);
        }

        for (i, ports) in mgr.iter().enumerate() {
            for signal in ["ack", "err", "rty"] {
                let gate = b.net(busy);
                let mine = b.net(grant_is[i]);
                let both = b.and(gate, mine);
                let value = b.net(shared[signal]);
                let value = b.and(both, value);
                b.assign(ports[signal], value);
            }
            let value = b.net(shared["dat_r"]);
            b.assign(ports["dat_r"], value);
        }

        // The granted manager's `cyc`, which holds the grant.
        let held = b.add_net("granted_cyc", Type::bit());
        let arms: Vec<ExprId> = mgr.iter().map(|p| b.net(p["cyc"])).collect();
        let default = b.const_bit(false);
        let value = select(&mut b, grant, grant_w, &arms, default);
        b.assign(held, value);

        let zero_grant = b.const_u64(grant_w, 0);
        let false_ = b.const_bit(false);
        let true_ = b.const_bit(true);
        let mut reset = b.block();
        reset.nonblocking(grant, zero_grant);
        reset.nonblocking(rr, zero_grant);
        reset.nonblocking(busy, false_);

        let mut take = b.block();
        let g = b.net(nxt_grant);
        take.nonblocking(grant, g);
        let g = b.net(nxt_grant);
        take.nonblocking(rr, g);
        take.nonblocking(busy, true_);
        let mut idle = b.block();
        let cond = b.net(any_req);
        idle.if_(cond, take.finish(), Vec::new());

        let mut release = b.block();
        release.nonblocking(busy, false_);
        let mut running = b.block();
        let still = b.net(held);
        running.if_(still, Vec::new(), release.finish());

        let mut body = b.block();
        let is_busy = b.net(busy);
        body.if_(is_busy, running.finish(), idle.finish());

        let mut process = b.process(
            Some("arbiter"),
            ProcessKind::Sequential {
                clocks: vec![Edge::pos(clk)],
                resets: vec![Edge::neg(rst_n)],
            },
        );
        let r = b.net(rst_n);
        let in_reset = b.lnot(r);
        process.if_(in_reset, reset.finish(), body.finish());
        b.end_process(process);

        b.finish()
    }
}

// ---------------------------------------------------------------------------
// Shared building blocks
// ---------------------------------------------------------------------------

/// The port-name prefixes for `count` interfaces tagged `<tag><n>_`.
fn prefixes(tag: &str, count: usize) -> Vec<String> {
    (0..count).map(|i| format!("{tag}{i}_")).collect()
}

/// Adds one copy of `interface`'s ports per prefix, and returns each
/// copy's ports by signal name.
fn add_bus_ports(
    b: &mut ModuleBuilder,
    interface: &BusInterface,
    role: BusRole,
    prefixes: &[String],
    bindings: &BTreeMap<String, u32>,
) -> Vec<BTreeMap<String, NetId>> {
    let mut out = Vec::with_capacity(prefixes.len());
    for prefix in prefixes {
        let mut ports = BTreeMap::new();
        for signal in &interface.signals {
            // A parameter the caller did not bind keeps the bus's own
            // default, which `resolve` has already applied; a width that
            // still does not resolve can only be one bit.
            let width = signal.width.resolve(bindings).unwrap_or(1);
            let name = format!("{prefix}{}", signal.name);
            let net = match signal.direction(role) {
                PortDir::In => b.input(name, Type::bits(width)),
                PortDir::Out => b.output(name, Type::bits(width)),
                PortDir::InOut => b.inout(name, Type::bits(width)),
            };
            ports.insert(signal.name.clone(), net);
        }
        out.push(ports);
    }
    out
}

/// `arms[index]`, as a chain of comparisons ending in `default`.
///
/// A chain rather than a one-hot tree because the result has to be read
/// by a person: this is what a `case` statement would lower to, and
/// synthesis turns it back into one.
fn select(
    b: &mut ModuleBuilder,
    index: NetId,
    index_width: u32,
    arms: &[ExprId],
    default: ExprId,
) -> ExprId {
    let mut out = default;
    for (value, arm) in arms.iter().enumerate().rev() {
        let i = b.net(index);
        let k = b.const_u64(index_width, idx(value));
        let hit = b.eq(i, k);
        out = b.mux(hit, *arm, out);
    }
    out
}

/// The OR of every net, or a constant zero when there are none.
fn reduce_or(b: &mut ModuleBuilder, nets: &[NetId]) -> ExprId {
    let mut out = None;
    for net in nets {
        let value = b.net(*net);
        out = Some(match out {
            None => value,
            Some(acc) => b.or(acc, value),
        });
    }
    match out {
        Some(expr) => expr,
        None => b.const_bit(false),
    }
}

/// The index of the first requester strictly after `last`, wrapping.
///
/// One priority chain per possible value of `last`, selected by `last`
/// itself. That is `O(n²)` gates, which for the handful of managers a
/// real interconnect has is both small and easy to read; a one-hot
/// rotate-and-mask version would be neither.
fn round_robin(b: &mut ModuleBuilder, last: NetId, reqs: &[NetId], width: u32) -> ExprId {
    let n = reqs.len();
    if n <= 1 {
        return b.const_u64(width, 0);
    }
    let mut chains = Vec::with_capacity(n);
    for start in 0..n {
        let mut chain = b.const_u64(width, idx(start));
        for step in (1..=n).rev() {
            let candidate = (start + step) % n;
            let value = b.const_u64(width, idx(candidate));
            let cond = b.net(reqs[candidate]);
            chain = b.mux(cond, value, chain);
        }
        chains.push(chain);
    }
    let default = b.const_u64(width, 0);
    select(b, last, width, &chains, default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Design;
    use crate::ir::validate::validate;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        Span::new(map.add("interconnect-test", "").unwrap(), 0, 0)
    }

    fn ranges() -> Vec<AddressRange> {
        vec![
            AddressRange::new(0x0000, 0x1000),
            AddressRange::new(0x1000, 0x1000),
        ]
    }

    #[test]
    fn index_widths_are_minimal() {
        assert_eq!(idx_width(0), 1);
        assert_eq!(idx_width(1), 1);
        assert_eq!(idx_width(2), 1);
        assert_eq!(idx_width(3), 2);
        assert_eq!(idx_width(4), 2);
        assert_eq!(idx_width(5), 3);
        assert_eq!(idx_width(256), 8);
        assert_eq!(idx_width(257), 9);
    }

    #[test]
    fn address_ranges_round_up_and_decode() {
        let r = AddressRange::new(0x2000, 0x1000);
        assert_eq!(r.end(), 0x3000);
        assert!(r.contains(0x2000));
        assert!(r.contains(0x2fff));
        assert!(!r.contains(0x1fff));
        assert!(!r.contains(0x3000));
        assert!(r.is_aligned());
        assert!(!AddressRange::new(0x2800, 0x1000).is_aligned());
        assert_eq!(AddressRange::new(0, 3).size, 4);
        assert_eq!(AddressRange::new(0, 0x1000).mask(), !0xfffu64);
    }

    #[test]
    fn a_bad_configuration_says_so() {
        let bad = Crossbar::new(
            "x",
            0,
            vec![
                AddressRange::new(0x800, 0x1000),
                AddressRange::new(0x1000, 0x1000),
            ],
            span(),
        )
        .with_widths(0, 12);
        let problems = bad.problems();
        assert!(
            problems.iter().any(|p| p.contains("no managers")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("outside 1..=64")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("whole number of bytes")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("not a multiple of its size")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("overlap")),
            "{problems:?}"
        );

        let narrow =
            Crossbar::new("x", 1, vec![AddressRange::new(0, 0x10000)], span()).with_widths(8, 32);
        assert!(
            narrow
                .problems()
                .iter()
                .any(|p| p.contains("past the 8-bit")),
            "{:?}",
            narrow.problems()
        );
        assert!(
            Crossbar::new("x", 2, ranges(), span())
                .problems()
                .is_empty()
        );
        assert!(
            Crossbar::new("x", 1, Vec::new(), span())
                .problems()
                .iter()
                .any(|p| p.contains("DECERR to everything"))
        );
    }

    #[test]
    fn the_crossbar_validates_and_has_the_interfaces_it_claims() {
        let module = Crossbar::new("xbar", 3, ranges(), span()).build();
        let mut design = Design::new();
        design.top = Some(design.add_module(module));
        let diags = validate(&design);
        assert!(diags.is_empty(), "{:?}", diags.iter().next());

        let module = design.top_module().unwrap();
        let axi = bus::builtin("axi4lite").unwrap();
        for i in 0..3 {
            bus::match_ports(module, axi, BusRole::Subordinate, &format!("s{i}_"))
                .unwrap_or_else(|p| panic!("manager {i}: {p:?}"));
        }
        for j in 0..2 {
            bus::match_ports(module, axi, BusRole::Manager, &format!("m{j}_"))
                .unwrap_or_else(|p| panic!("subordinate {j}: {p:?}"));
        }
        assert_eq!(
            module.attrs.get("generator").and_then(|v| v.as_str()),
            Some("reticle::ip::interconnect::Crossbar")
        );
        assert_eq!(module.param("MANAGERS").unwrap().value.as_int(), Some(3));
    }

    #[test]
    fn the_crossbar_round_trips_through_the_text_format() {
        let module = Crossbar::new("xbar", 2, ranges(), span()).build();
        let mut design = Design::new();
        design.top = Some(design.add_module(module));
        let text = design.to_text();
        let mut map = SourceMap::new();
        let file = map.add("xbar.rtl", &text).unwrap();
        let again = Design::parse_text(&text, file).expect("parses");
        assert_eq!(again.to_text(), text);
    }

    #[test]
    fn a_wide_crossbar_uses_the_widths_it_was_given() {
        let module = Crossbar::new("xbar", 1, ranges(), span())
            .with_widths(16, 64)
            .build();
        let net = |name: &str| {
            let port = module.port(name).expect(name);
            module.nets[port.net].ty.width().unwrap()
        };
        assert_eq!(net("s0_awaddr"), 16);
        assert_eq!(net("s0_wdata"), 64);
        assert_eq!(net("s0_wstrb"), 8);
        assert_eq!(net("m0_rdata"), 64);
    }

    #[test]
    fn the_wishbone_arbiter_validates_and_matches_its_bus() {
        let module = WishboneArbiter::new("wb_arb", 2, span()).build();
        let mut design = Design::new();
        design.top = Some(design.add_module(module));
        assert!(validate(&design).is_empty());
        let module = design.top_module().unwrap();
        let wb = bus::builtin("wishbone").unwrap();
        bus::match_ports(module, wb, BusRole::Subordinate, "s0_").unwrap();
        bus::match_ports(module, wb, BusRole::Subordinate, "s1_").unwrap();
        bus::match_ports(module, wb, BusRole::Manager, "m_").unwrap();
    }

    #[test]
    fn degenerate_configurations_still_build() {
        for (n, m) in [(0usize, 0usize), (1, 0), (0, 1), (1, 1)] {
            let subordinates = (0..m)
                .map(|j| AddressRange::new(idx(j) * 0x1000, 0x1000))
                .collect();
            let module = Crossbar::new("x", n, subordinates, span()).build();
            let mut design = Design::new();
            design.top = Some(design.add_module(module));
            assert!(validate(&design).is_empty(), "{n}x{m}");
        }
        let module = WishboneArbiter::new("a", 0, span()).build();
        let mut design = Design::new();
        design.top = Some(design.add_module(module));
        assert!(validate(&design).is_empty());
    }
}

/// Running the generated interconnect.
///
/// These are the tests that matter: a netlist that looks right and does
/// the wrong thing is the failure mode a generator has. Each builds a
/// system out of the generated module and real subordinates, wires it
/// with [`super::bus::connect`], and drives AXI transactions through the
/// event-driven simulator.
#[cfg(all(test, feature = "sim"))]
mod sim_tests {
    use super::*;
    use crate::diag::Diagnostics;
    use crate::ip::bus::{BusEndpoint, connect, wire};
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::validate::validate;
    use crate::ir::{Delay, Design, ModuleRef, Name, TimeUnit};
    use crate::logic::Logic;
    use crate::sim::{SimOptions, Simulator};
    use crate::source::{SourceMap, Span};
    use crate::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

    /// A minimal AXI4-Lite subordinate: four registers at `addr[3:2]`.
    ///
    /// Kept here rather than in `testdata/` because it is a fixture of
    /// this test and nothing else; the IP package under
    /// `testdata/ip/axi_regs/` is the same component wrapped as a real
    /// package, which `tests/ip_project.rs` uses.
    const SLAVE: &str = r"
module axil_regs(
  input             clk,
  input             rst_n,
  input      [31:0] s_awaddr,
  input       [2:0] s_awprot,
  input             s_awvalid,
  output            s_awready,
  input      [31:0] s_wdata,
  input       [3:0] s_wstrb,
  input             s_wvalid,
  output            s_wready,
  output      [1:0] s_bresp,
  output            s_bvalid,
  input             s_bready,
  input      [31:0] s_araddr,
  input       [2:0] s_arprot,
  input             s_arvalid,
  output            s_arready,
  output     [31:0] s_rdata,
  output      [1:0] s_rresp,
  output            s_rvalid,
  input             s_rready
);
  reg [31:0] r0, r1, r2, r3;
  reg [31:0] awaddr_q, wdata_q, rdata_q;
  reg        aw_seen, w_seen, bvalid_q, rvalid_q;

  assign s_awready = !aw_seen && !bvalid_q;
  assign s_wready  = !w_seen  && !bvalid_q;
  assign s_bvalid  = bvalid_q;
  assign s_bresp   = 2'b00;
  assign s_arready = !rvalid_q;
  assign s_rvalid  = rvalid_q;
  assign s_rdata   = rdata_q;
  assign s_rresp   = 2'b00;

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      r0 <= 32'd0; r1 <= 32'd0; r2 <= 32'd0; r3 <= 32'd0;
      awaddr_q <= 32'd0; wdata_q <= 32'd0; rdata_q <= 32'd0;
      aw_seen <= 1'b0; w_seen <= 1'b0; bvalid_q <= 1'b0; rvalid_q <= 1'b0;
    end else begin
      if (s_awvalid && s_awready) begin
        aw_seen  <= 1'b1;
        awaddr_q <= s_awaddr;
      end
      if (s_wvalid && s_wready) begin
        w_seen  <= 1'b1;
        wdata_q <= s_wdata;
      end
      if (aw_seen && w_seen && !bvalid_q) begin
        case (awaddr_q[3:2])
          2'd0: r0 <= wdata_q;
          2'd1: r1 <= wdata_q;
          2'd2: r2 <= wdata_q;
          default: r3 <= wdata_q;
        endcase
        aw_seen  <= 1'b0;
        w_seen   <= 1'b0;
        bvalid_q <= 1'b1;
      end
      if (bvalid_q && s_bready) bvalid_q <= 1'b0;
      if (s_arvalid && s_arready) begin
        case (s_araddr[3:2])
          2'd0: rdata_q <= r0;
          2'd1: rdata_q <= r1;
          2'd2: rdata_q <= r2;
          default: rdata_q <= r3;
        endcase
        rvalid_q <= 1'b1;
      end else if (rvalid_q && s_rready) begin
        rvalid_q <= 1'b0;
      end
    end
  end
endmodule
";

    /// The inputs of one manager-facing interface, all driven low at
    /// reset so the arbiter never sees an `x` request.
    const MANAGER_INPUTS: [(&str, u32); 11] = [
        ("awaddr", 32),
        ("awprot", 3),
        ("awvalid", 1),
        ("wdata", 32),
        ("wstrb", 4),
        ("wvalid", 1),
        ("bready", 1),
        ("araddr", 32),
        ("arprot", 3),
        ("arvalid", 1),
        ("rready", 1),
    ];

    /// Builds `top`: a crossbar with `managers` manager interfaces, two
    /// `axil_regs` subordinates at 0x0000 and 0x1000, wired with
    /// [`connect`], and every manager interface brought out to a port.
    fn system(managers: usize) -> Design {
        let mut map = SourceMap::new();
        let id = map.add("axil_regs.v", SLAVE).unwrap();
        let mut diags = Diagnostics::new();
        let file = parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        );
        let mut design = elaborate(&[&file], &ElabOptions::default(), &mut diags)
            .expect("the subordinate elaborates");
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        let span = Span::new(id, 0, 0);
        let slave = design.module_by_name("axil_regs").expect("the subordinate");

        let ranges = vec![
            AddressRange::new(0x0000, 0x1000),
            AddressRange::new(0x1000, 0x1000),
        ];
        let crossbar = Crossbar::new("axil_xbar", managers, ranges, span);
        assert!(crossbar.problems().is_empty());
        let xbar_module = crossbar.build();
        let xbar_ports: Vec<(Name, PortDir, u32)> = xbar_module
            .ports
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    p.dir,
                    xbar_module.nets[p.net].ty.width().unwrap_or(1),
                )
            })
            .collect();
        let xbar = design.add_module(xbar_module);

        let mut b = ModuleBuilder::new("top", span);
        let clk = b.input("clk", Type::bit());
        let rst_n = b.input("rst_n", Type::bit());
        let clk_expr = b.net(clk);
        let rst_expr = b.net(rst_n);
        let mut connections = vec![(Name::new("clk"), clk_expr), (Name::new("rst_n"), rst_expr)];
        for (name, dir, width) in &xbar_ports {
            if !name.as_str().starts_with('s') {
                continue;
            }
            let net = match dir {
                PortDir::In => b.input(name.clone(), Type::bits(*width)),
                _ => b.output(name.clone(), Type::bits(*width)),
            };
            let expr = b.net(net);
            connections.push((name.clone(), expr));
        }
        let u_xbar = b.instance("u_xbar", ModuleRef::Resolved(xbar), connections);
        let mut slaves = Vec::new();
        for j in 0..2 {
            let clk_expr = b.net(clk);
            let rst_expr = b.net(rst_n);
            slaves.push(b.instance(
                format!("u_s{j}"),
                ModuleRef::Resolved(slave),
                vec![(Name::new("clk"), clk_expr), (Name::new("rst_n"), rst_expr)],
            ));
        }
        let top = design.add_module(b.finish());
        design.top = Some(top);

        let axi = bus::builtin("axi4lite").unwrap();
        let mut wiring = Vec::new();
        for (j, slave_instance) in slaves.iter().enumerate() {
            let links = connect(
                &design,
                top,
                &BusEndpoint::new(u_xbar, format!("m{j}_")),
                &BusEndpoint::new(*slave_instance, "s_"),
                axi,
            )
            .unwrap_or_else(|problems| panic!("wiring subordinate {j}: {problems:?}"));
            assert_eq!(links.len(), axi.signals.len());
            wiring.push((*slave_instance, links));
        }
        let module = design.module(top).clone();
        let mut b = ModuleBuilder::from_module(module, span);
        for (slave_instance, links) in &wiring {
            wire(&mut b, u_xbar, *slave_instance, links);
        }
        *design.module_mut(top) = b.finish();

        let diags = validate(&design);
        assert!(diags.is_empty(), "{:?}", diags.iter().next());
        design
    }

    /// One transaction driven at one manager.
    #[derive(Clone, Copy, Debug)]
    enum Op {
        Write { addr: u64, data: u64 },
        Read { addr: u64 },
    }

    #[derive(Debug)]
    struct Txn {
        manager: usize,
        op: Op,
        addr_done: bool,
        data_done: bool,
        finished: bool,
        /// The response code, once the transaction ends.
        resp: u64,
        /// The data read, for a read.
        data: u64,
    }

    impl Txn {
        fn new(manager: usize, op: Op) -> Self {
            Txn {
                manager,
                op,
                addr_done: false,
                data_done: false,
                finished: false,
                resp: 0,
                data: 0,
            }
        }
    }

    /// The whole testbench: handles, clocking and the AXI driver.
    struct Bench<'d> {
        sim: Simulator<'d>,
        half: u64,
    }

    impl<'d> Bench<'d> {
        fn new(design: &'d Design, managers: usize) -> Self {
            let sim = Simulator::new(design, SimOptions::default()).expect("elaborates");
            let half = sim.ticks(Delay::new(5, TimeUnit::Ns));
            let mut bench = Bench { sim, half };
            bench.sim.run_for(0);
            bench.put("clk", 1, 0);
            for i in 0..managers {
                for (signal, width) in MANAGER_INPUTS {
                    bench.put(&format!("s{i}_{signal}"), width, 0);
                }
            }
            bench.put("rst_n", 1, 0);
            for _ in 0..3 {
                bench.step();
            }
            bench.put("rst_n", 1, 1);
            bench.step();
            bench
        }

        fn put(&mut self, name: &str, width: u32, value: u64) {
            let handle = self
                .sim
                .net(&format!("top.{name}"))
                .unwrap_or_else(|| panic!("no net `{name}`"));
            self.sim.set(handle, Logic::from_u64(value, width));
        }

        fn get(&self, name: &str) -> u64 {
            let handle = self
                .sim
                .net(&format!("top.{name}"))
                .unwrap_or_else(|| panic!("no net `{name}`"));
            self.sim.get(handle).to_u64().unwrap_or(u64::MAX)
        }

        fn step(&mut self) {
            self.put("clk", 1, 1);
            self.sim.run_for(self.half);
            self.put("clk", 1, 0);
            self.sim.run_for(self.half);
        }

        /// Drives every transaction at once and returns the order in
        /// which they completed.
        fn run(&mut self, txns: &mut [Txn]) -> Vec<usize> {
            for txn in txns.iter() {
                let i = txn.manager;
                match txn.op {
                    Op::Write { addr, data } => {
                        self.put(&format!("s{i}_awaddr"), 32, addr);
                        self.put(&format!("s{i}_awvalid"), 1, 1);
                        self.put(&format!("s{i}_wdata"), 32, data);
                        self.put(&format!("s{i}_wstrb"), 4, 0xf);
                        self.put(&format!("s{i}_wvalid"), 1, 1);
                        self.put(&format!("s{i}_bready"), 1, 1);
                    }
                    Op::Read { addr } => {
                        self.put(&format!("s{i}_araddr"), 32, addr);
                        self.put(&format!("s{i}_arvalid"), 1, 1);
                        self.put(&format!("s{i}_rready"), 1, 1);
                    }
                }
            }
            let mut order = Vec::new();
            for _ in 0..200 {
                if txns.iter().all(|t| t.finished) {
                    break;
                }
                // Sample every handshake as it stands before the edge,
                // which is what the flip-flops will see.
                let samples: Vec<[u64; 5]> = txns
                    .iter()
                    .map(|t| {
                        let i = t.manager;
                        match t.op {
                            Op::Write { .. } => [
                                self.get(&format!("s{i}_awready")),
                                self.get(&format!("s{i}_wready")),
                                self.get(&format!("s{i}_bvalid")),
                                self.get(&format!("s{i}_bresp")),
                                0,
                            ],
                            Op::Read { .. } => [
                                self.get(&format!("s{i}_arready")),
                                0,
                                self.get(&format!("s{i}_rvalid")),
                                self.get(&format!("s{i}_rresp")),
                                self.get(&format!("s{i}_rdata")),
                            ],
                        }
                    })
                    .collect();
                self.step();
                for (index, txn) in txns.iter_mut().enumerate() {
                    if txn.finished {
                        continue;
                    }
                    let [first, second, valid, resp, data] = samples[index];
                    let i = txn.manager;
                    let mut writes: Vec<(String, u32, u64)> = Vec::new();
                    match txn.op {
                        Op::Write { .. } => {
                            if !txn.addr_done && first == 1 {
                                txn.addr_done = true;
                                writes.push((format!("s{i}_awvalid"), 1, 0));
                            }
                            if !txn.data_done && second == 1 {
                                txn.data_done = true;
                                writes.push((format!("s{i}_wvalid"), 1, 0));
                            }
                            if valid == 1 {
                                txn.finished = true;
                                txn.resp = resp;
                                writes.push((format!("s{i}_bready"), 1, 0));
                            }
                        }
                        Op::Read { .. } => {
                            if !txn.addr_done && first == 1 {
                                txn.addr_done = true;
                                writes.push((format!("s{i}_arvalid"), 1, 0));
                            }
                            if valid == 1 {
                                txn.finished = true;
                                txn.resp = resp;
                                txn.data = data;
                                writes.push((format!("s{i}_rready"), 1, 0));
                            }
                        }
                    }
                    let done = txn.finished;
                    for (name, width, value) in writes {
                        self.put(&name, width, value);
                    }
                    if done {
                        order.push(index);
                    }
                }
            }
            assert!(
                txns.iter().all(|t| t.finished),
                "a transaction never completed: {txns:?}"
            );
            order
        }

        fn write(&mut self, manager: usize, addr: u64, data: u64) -> u64 {
            let mut txns = [Txn::new(manager, Op::Write { addr, data })];
            self.run(&mut txns);
            txns[0].resp
        }

        fn read(&mut self, manager: usize, addr: u64) -> (u64, u64) {
            let mut txns = [Txn::new(manager, Op::Read { addr })];
            self.run(&mut txns);
            (txns[0].data, txns[0].resp)
        }
    }

    #[test]
    fn the_crossbar_routes_writes_and_reads_by_address() {
        let design = system(2);
        let mut bench = Bench::new(&design, 2);

        // Subordinate 0 at 0x0000, register 1; subordinate 1 at 0x1000,
        // register 2. The last pair uses the same register index in both
        // subordinates on purpose, so only the decode tells them apart.
        assert_eq!(bench.write(0, 0x0004, 0xa5a5_0001), 0);
        assert_eq!(bench.write(0, 0x1008, 0x5a5a_0002), 0);
        assert_eq!(bench.write(0, 0x0008, 0x0000_0003), 0);

        assert_eq!(bench.read(0, 0x0004), (0xa5a5_0001, 0));
        assert_eq!(bench.read(0, 0x1008), (0x5a5a_0002, 0));
        assert_eq!(bench.read(0, 0x0008), (0x0000_0003, 0));
        // Never written, and in the *other* subordinate's register 1.
        assert_eq!(bench.read(0, 0x1004), (0, 0));
    }

    #[test]
    fn an_unmapped_address_is_answered_with_decerr() {
        let design = system(1);
        let mut bench = Bench::new(&design, 1);
        assert_eq!(bench.read(0, 0x9000), (0, DECERR));
        assert_eq!(bench.write(0, 0x9000, 0xdead_beef), DECERR);
        // And the bus still works afterwards: the error responder handed
        // the grant back.
        assert_eq!(bench.write(0, 0x0000, 0x1234_5678), 0);
        assert_eq!(bench.read(0, 0x0000), (0x1234_5678, 0));
    }

    #[test]
    fn both_managers_reach_both_subordinates() {
        let design = system(2);
        let mut bench = Bench::new(&design, 2);
        assert_eq!(bench.write(1, 0x0000, 0x1111_1111), 0);
        assert_eq!(bench.write(1, 0x100c, 0x2222_2222), 0);
        // Manager 0 sees what manager 1 wrote, through the same
        // subordinates.
        assert_eq!(bench.read(0, 0x0000), (0x1111_1111, 0));
        assert_eq!(bench.read(0, 0x100c), (0x2222_2222, 0));
        assert_eq!(bench.read(1, 0x100c), (0x2222_2222, 0));
    }

    #[test]
    fn arbitration_alternates_between_the_managers() {
        let design = system(2);
        let mut bench = Bench::new(&design, 2);
        // Prime the round-robin pointer with a transaction from 0.
        assert_eq!(bench.write(0, 0x0000, 0x0000_00aa), 0);

        // Both managers ask at the same instant. The pointer is at 0,
        // so the manager after it goes first.
        let mut txns = [
            Txn::new(0, Op::Read { addr: 0x0000 }),
            Txn::new(1, Op::Read { addr: 0x0000 }),
        ];
        assert_eq!(bench.run(&mut txns), [1, 0]);
        assert_eq!(txns[0].data, 0xaa);
        assert_eq!(txns[1].data, 0xaa);

        // Move the pointer to 1 and the order reverses, which is what
        // makes the arbitration round-robin rather than fixed priority.
        assert_eq!(bench.read(1, 0x0000), (0xaa, 0));
        let mut txns = [
            Txn::new(0, Op::Read { addr: 0x0000 }),
            Txn::new(1, Op::Read { addr: 0x0000 }),
        ];
        assert_eq!(bench.run(&mut txns), [0, 1]);
    }

    /// Two Wishbone managers onto one shared bus, with a subordinate
    /// that acknowledges every cycle.
    fn wishbone_system() -> Design {
        const SUB: &str = r"
module wb_ram(
  input         clk,
  input         rst_n,
  input  [31:0] s_adr,
  input  [31:0] s_dat_w,
  output [31:0] s_dat_r,
  input   [3:0] s_sel,
  input         s_we,
  input         s_stb,
  input         s_cyc,
  output        s_ack,
  output        s_err,
  output        s_rty
);
  reg [31:0] word_q;
  reg        ack_q;
  assign s_ack   = ack_q;
  assign s_err   = 1'b0;
  assign s_rty   = 1'b0;
  assign s_dat_r = word_q;
  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      word_q <= 32'd0;
      ack_q <= 1'b0;
    end else begin
      if (s_cyc && s_stb && !ack_q) begin
        if (s_we) word_q <= s_dat_w;
        ack_q <= 1'b1;
      end else begin
        ack_q <= 1'b0;
      end
    end
  end
endmodule
";
        let mut map = SourceMap::new();
        let id = map.add("wb_ram.v", SUB).unwrap();
        let mut diags = Diagnostics::new();
        let file = parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        );
        let design = elaborate(&[&file], &ElabOptions::default(), &mut diags);
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        let mut design = design.expect("elaborates");
        let span = Span::new(id, 0, 0);
        let ram = design.module_by_name("wb_ram").expect("the subordinate");

        let arbiter_module = WishboneArbiter::new("wb_arb", 2, span).build();
        let arbiter_ports: Vec<(Name, PortDir, u32)> = arbiter_module
            .ports
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    p.dir,
                    arbiter_module.nets[p.net].ty.width().unwrap_or(1),
                )
            })
            .collect();
        let arbiter = design.add_module(arbiter_module);

        let mut b = ModuleBuilder::new("top", span);
        let clk = b.input("clk", Type::bit());
        let rst_n = b.input("rst_n", Type::bit());
        let clk_expr = b.net(clk);
        let rst_expr = b.net(rst_n);
        let mut connections = vec![(Name::new("clk"), clk_expr), (Name::new("rst_n"), rst_expr)];
        for (name, dir, width) in &arbiter_ports {
            if !name.as_str().starts_with('s') {
                continue;
            }
            let net = match dir {
                PortDir::In => b.input(name.clone(), Type::bits(*width)),
                _ => b.output(name.clone(), Type::bits(*width)),
            };
            let expr = b.net(net);
            connections.push((name.clone(), expr));
        }
        let u_arb = b.instance("u_arb", ModuleRef::Resolved(arbiter), connections);
        let clk_expr = b.net(clk);
        let rst_expr = b.net(rst_n);
        let u_ram = b.instance(
            "u_ram",
            ModuleRef::Resolved(ram),
            vec![(Name::new("clk"), clk_expr), (Name::new("rst_n"), rst_expr)],
        );
        let top = design.add_module(b.finish());
        design.top = Some(top);

        let wb = bus::builtin("wishbone").unwrap();
        let links = connect(
            &design,
            top,
            &BusEndpoint::new(u_arb, "m_"),
            &BusEndpoint::new(u_ram, "s_"),
            wb,
        )
        .unwrap_or_else(|problems| panic!("wiring the shared bus: {problems:?}"));
        let module = design.module(top).clone();
        let mut b = ModuleBuilder::from_module(module, span);
        wire(&mut b, u_arb, u_ram, &links);
        *design.module_mut(top) = b.finish();
        assert!(validate(&design).is_empty());
        design
    }

    #[test]
    fn the_wishbone_arbiter_hands_the_bus_over_in_turn() {
        let design = wishbone_system();
        let sim = Simulator::new(&design, SimOptions::default()).expect("elaborates");
        let half = sim.ticks(Delay::new(5, TimeUnit::Ns));
        let mut bench = Bench { sim, half };
        bench.sim.run_for(0);
        bench.put("clk", 1, 0);
        for i in 0..2 {
            for (signal, width) in [
                ("adr", 32u32),
                ("dat_w", 32),
                ("sel", 4),
                ("we", 1),
                ("stb", 1),
                ("cyc", 1),
            ] {
                bench.put(&format!("s{i}_{signal}"), width, 0);
            }
        }
        bench.put("rst_n", 1, 0);
        for _ in 0..3 {
            bench.step();
        }
        bench.put("rst_n", 1, 1);
        bench.step();

        // Both managers raise `cyc` in the same cycle; manager 1 is next
        // after the reset pointer, so it is served first.
        for i in 0..2 {
            bench.put(&format!("s{i}_cyc"), 1, 1);
            bench.put(&format!("s{i}_stb"), 1, 1);
            bench.put(&format!("s{i}_we"), 1, 1);
            bench.put(&format!("s{i}_sel"), 4, 0xf);
        }
        bench.put("s0_dat_w", 32, 0x0000_aaaa);
        bench.put("s1_dat_w", 32, 0x0000_bbbb);

        let mut served: Vec<usize> = Vec::new();
        for _ in 0..40 {
            let acks = [bench.get("s0_ack"), bench.get("s1_ack")];
            bench.step();
            for (i, ack) in acks.into_iter().enumerate() {
                if ack == 1 && !served.contains(&i) {
                    served.push(i);
                    bench.put(&format!("s{i}_cyc"), 1, 0);
                    bench.put(&format!("s{i}_stb"), 1, 0);
                }
            }
            if served.len() == 2 {
                break;
            }
        }
        assert_eq!(served, [1, 0], "the arbiter must serve 1 then 0");
        // The last writer wins the shared cell, which is manager 0.
        assert_eq!(bench.get("s0_dat_r"), 0x0000_aaaa);
    }
}
