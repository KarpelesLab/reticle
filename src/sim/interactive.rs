//! Interactive mode: a command interpreter over a [`Simulator`].
//!
//! [`Session`] is sans-I/O, like everything else under `src/`: it takes a
//! command line as a `&str` and returns the text to show as a
//! [`Response`], so the same session drives a terminal, a socket, a test
//! or an editor plug-in. The CLI is one loop around
//! [`Session::execute`]; [`Session::complete`] supplies the candidates for
//! a tab key.
//!
//! # Commands
//!
//! | Command                   | What it does                              |
//! |---------------------------|-------------------------------------------|
//! | `run [time]`              | run for `time`, or to the end             |
//! | `step`                    | run one time slot                         |
//! | `continue`                | run to the end, a breakpoint or `$stop`   |
//! | `break <net> [value]`     | stop when the net changes (to `value`)    |
//! | `delete [id]`             | remove one breakpoint, or all of them     |
//! | `force <net> <value>`     | override a net                            |
//! | `release <net>`           | drop the override                         |
//! | `print <net>`             | show a net's value                        |
//! | `watch <net>`             | report every change of a net              |
//! | `dump on` / `dump off`    | start or stop VCD capture                 |
//! | `memory <name>[<index>]`  | show memory contents                      |
//! | `scope [path]`            | show or set the scope names resolve in    |
//! | `where`                   | scope, module, time and run state         |
//! | `time`                    | the current time                          |
//! | `help`                    | the command list                          |
//! | `quit`                    | end the session                           |
//!
//! Times are in ticks of the simulation precision, or carry a unit
//! (`100ns`, `5us`). Values are Verilog literals (`1'b1`, `8'hff`, `42`).
//! A net name is hierarchical (`top.fifo.full`) or relative to the current
//! scope (`full`).
//!
//! # Breakpoints
//!
//! A breakpoint is on a net *changing*, optionally to a given value. It is
//! checked where the simulator propagates a change, and the run stops at
//! the end of that time slot rather than in the middle of it, so the
//! design is in a consistent state when the prompt comes back: every
//! region of the slot has run. `run` and `continue` report which
//! breakpoints were hit before their status line.
//!
//! # Watches
//!
//! A watch prints one line per change, with the time, so a `run` reports
//! what happened while it ran rather than only where it ended. Watch lines
//! come first in a response, before the command's own output. Watches
//! cannot be removed: a session that wants fewer of them starts again.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::ir::{Delay, TimeUnit};
use crate::logic::Logic;

use super::{BreakId, MemHandle, NetHandle, Simulator, Status};

/// Every command name, for `help` and for completion.
const COMMANDS: &[&str] = &[
    "break", "continue", "delete", "dump", "force", "help", "memory", "print", "quit", "release",
    "run", "scope", "step", "time", "watch", "where",
];

/// How many memory elements `memory <name>` shows without a range.
const MEMORY_PREVIEW: usize = 16;

/// What a command produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// The lines to show, without trailing newlines.
    pub lines: Vec<String>,
    /// True after `quit`: the driver should stop reading commands.
    pub quit: bool,
}

impl Response {
    /// A response of one line.
    fn line(text: impl Into<String>) -> Response {
        Response {
            lines: vec![text.into()],
            quit: false,
        }
    }

    /// The lines joined by newlines, with a trailing newline when there is
    /// anything to show.
    pub fn text(&self) -> String {
        if self.lines.is_empty() {
            return String::new();
        }
        let mut out = self.lines.join("\n");
        out.push('\n');
        out
    }
}

/// Why a command could not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The command word is not one the session knows; `help` lists them.
    UnknownCommand(String),
    /// The arguments do not fit the command; carries the usage line.
    Usage(&'static str),
    /// No net of that name, in the scope or at the top.
    UnknownNet(String),
    /// No memory of that name.
    UnknownMemory(String),
    /// A value that is not a Verilog literal.
    BadValue(String),
    /// A time that is not a number with an optional unit.
    BadTime(String),
    /// A memory index that is not a number or a `lo:hi` range.
    BadIndex(String),
    /// No instance at that hierarchical path.
    UnknownScope(String),
    /// No breakpoint with that id.
    UnknownBreakpoint(u32),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::UnknownCommand(c) => {
                write!(f, "unknown command `{c}`; `help` lists them")
            }
            SessionError::Usage(u) => write!(f, "usage: {u}"),
            SessionError::UnknownNet(n) => write!(f, "unknown net `{n}`"),
            SessionError::UnknownMemory(n) => write!(f, "unknown memory `{n}`"),
            SessionError::BadValue(v) => write!(f, "`{v}` is not a value"),
            SessionError::BadTime(t) => write!(f, "`{t}` is not a time"),
            SessionError::BadIndex(i) => write!(f, "`{i}` is not an index or a range"),
            SessionError::UnknownScope(s) => write!(f, "unknown scope `{s}`"),
            SessionError::UnknownBreakpoint(id) => write!(f, "no breakpoint {id}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// A command session over one simulation.
///
/// See the [module docs](self) for the command language.
pub struct Session<'d> {
    sim: Simulator<'d>,
    scope: String,
    watched: Vec<String>,
    log: Rc<RefCell<Vec<String>>>,
    dump: Option<String>,
}

impl fmt::Debug for Session<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("scope", &self.scope)
            .field("time", &self.sim.time())
            .field("status", &self.sim.status())
            .finish_non_exhaustive()
    }
}

impl<'d> Session<'d> {
    /// Starts a session on `sim`, with the scope at the top instance.
    pub fn new(sim: Simulator<'d>) -> Session<'d> {
        let scope = sim.top_name().to_owned();
        Session {
            sim,
            scope,
            watched: Vec::new(),
            log: Rc::new(RefCell::new(Vec::new())),
            dump: None,
        }
    }

    /// The simulation being driven.
    pub fn simulator(&self) -> &Simulator<'d> {
        &self.sim
    }

    /// The simulation, mutably, for a driver that mixes commands with the
    /// co-simulation API.
    pub fn simulator_mut(&mut self) -> &mut Simulator<'d> {
        &mut self.sim
    }

    /// Gives the simulation back.
    pub fn into_simulator(self) -> Simulator<'d> {
        self.sim
    }

    /// The scope unqualified net names resolve in.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// The VCD text of the last `dump off`, if any.
    pub fn dump(&self) -> Option<&str> {
        self.dump.as_deref()
    }

    /// The help text `help` returns.
    pub fn help() -> &'static str {
        "\
run [time]              run for a time (`100ns`, `500`), or to the end
step                    run one time slot
continue                run to the end, a breakpoint or $stop
break <net> [value]     stop when a net changes, optionally to a value
delete [id]             remove a breakpoint, or every breakpoint
force <net> <value>     override a net until it is released
release <net>           drop an override
print <net>             show a net's value
watch <net>             report every change of a net
dump on|off             start or stop VCD capture
memory <name>[<i>]      show a memory, one element, or a `lo:hi` range
scope [path]            show or set the scope names resolve in
where                   scope, module, time and run state
time                    the current time in ticks
help                    this list
quit                    end the session"
    }

    /// Runs one command line.
    ///
    /// An empty line or a `#` comment is a no-op with an empty response.
    ///
    /// # Errors
    ///
    /// Returns a [`SessionError`] describing what was wrong with the
    /// command; the session stays usable.
    pub fn execute(&mut self, line: &str) -> Result<Response, SessionError> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return Ok(Response::default());
        }
        let words: Vec<&str> = line.split_whitespace().filter(|w| *w != "=").collect();
        let (command, args) = words.split_first().expect("non-empty");
        let mut response = self.dispatch(command, args)?;
        let mut lines: Vec<String> = self.log.borrow_mut().drain(..).collect();
        lines.append(&mut response.lines);
        response.lines = lines;
        Ok(response)
    }

    fn dispatch(&mut self, command: &str, args: &[&str]) -> Result<Response, SessionError> {
        match command {
            "run" => self.cmd_run(args),
            "continue" | "c" => self.cmd_run(&[]),
            "step" | "s" => self.cmd_step(),
            "break" | "b" => self.cmd_break(args),
            "delete" | "d" => self.cmd_delete(args),
            "force" => self.cmd_force(args),
            "release" => self.cmd_release(args),
            "print" | "p" => self.cmd_print(args),
            "watch" => self.cmd_watch(args),
            "dump" => self.cmd_dump(args),
            "memory" | "mem" => self.cmd_memory(args),
            "scope" => self.cmd_scope(args),
            "where" => self.cmd_where(),
            "time" => Ok(Response::line(format!("time = {}", self.sim.time()))),
            "help" | "h" | "?" => Ok(Response {
                lines: Session::help().lines().map(str::to_owned).collect(),
                quit: false,
            }),
            "quit" | "q" | "exit" => Ok(Response {
                lines: vec!["quit".to_owned()],
                quit: true,
            }),
            other => Err(SessionError::UnknownCommand(other.to_owned())),
        }
    }

    // ---- commands ----

    fn cmd_run(&mut self, args: &[&str]) -> Result<Response, SessionError> {
        match args {
            [] => self.sim.run(),
            [time] => {
                let ticks = self.ticks(time)?;
                self.sim.run_for(ticks);
            }
            _ => return Err(SessionError::Usage("run [time]")),
        }
        Ok(Response {
            lines: self.status_lines(false),
            quit: false,
        })
    }

    fn cmd_step(&mut self) -> Result<Response, SessionError> {
        self.sim.step();
        Ok(Response {
            lines: self.status_lines(true),
            quit: false,
        })
    }

    fn cmd_break(&mut self, args: &[&str]) -> Result<Response, SessionError> {
        let (name, value) = match args {
            [name] => (*name, None),
            [name, value] => (*name, Some(self.value(value)?)),
            _ => return Err(SessionError::Usage("break <net> [value]")),
        };
        let net = self.resolve(name)?;
        let id = self.sim.add_breakpoint(net, value.clone());
        let full = self.sim.net_name(net).to_owned();
        Ok(Response::line(match value {
            Some(v) => format!("breakpoint {} on {full} = {v}", id.number()),
            None => format!("breakpoint {} on {full}", id.number()),
        }))
    }

    fn cmd_delete(&mut self, args: &[&str]) -> Result<Response, SessionError> {
        match args {
            [] => {
                let n = self.sim.clear_breakpoints();
                Ok(Response::line(format!("deleted {n} breakpoints")))
            }
            [id] => {
                let id: u32 = id.parse().map_err(|_| SessionError::Usage("delete [id]"))?;
                if self.sim.remove_breakpoint(BreakId::from_number(id)) {
                    Ok(Response::line(format!("deleted breakpoint {id}")))
                } else {
                    Err(SessionError::UnknownBreakpoint(id))
                }
            }
            _ => Err(SessionError::Usage("delete [id]")),
        }
    }

    fn cmd_force(&mut self, args: &[&str]) -> Result<Response, SessionError> {
        let [name, value] = args else {
            return Err(SessionError::Usage("force <net> <value>"));
        };
        let value = self.value(value)?;
        let net = self.resolve(name)?;
        self.sim.force(net, value);
        let full = self.sim.net_name(net).to_owned();
        let now = self.sim.get(net);
        Ok(Response::line(format!("forced {full} = {now}")))
    }

    fn cmd_release(&mut self, args: &[&str]) -> Result<Response, SessionError> {
        let [name] = args else {
            return Err(SessionError::Usage("release <net>"));
        };
        let net = self.resolve(name)?;
        self.sim.release(net);
        let full = self.sim.net_name(net).to_owned();
        Ok(Response::line(format!("released {full}")))
    }

    fn cmd_print(&mut self, args: &[&str]) -> Result<Response, SessionError> {
        let [name] = args else {
            return Err(SessionError::Usage("print <net>"));
        };
        let net = self.resolve(name)?;
        Ok(Response::line(format!(
            "{} = {}",
            self.sim.net_name(net),
            self.sim.get(net)
        )))
    }

    fn cmd_watch(&mut self, args: &[&str]) -> Result<Response, SessionError> {
        let [name] = args else {
            return Err(SessionError::Usage("watch <net>"));
        };
        let net = self.resolve(name)?;
        let full = self.sim.net_name(net).to_owned();
        if self.watched.contains(&full) {
            return Ok(Response::line(format!("already watching {full}")));
        }
        self.watched.push(full.clone());
        let log = Rc::clone(&self.log);
        let label = full.clone();
        self.sim.on_change(net, move |time, value| {
            log.borrow_mut().push(format!("{time}: {label} = {value}"));
        });
        Ok(Response::line(format!("watching {full}")))
    }

    fn cmd_dump(&mut self, args: &[&str]) -> Result<Response, SessionError> {
        match args {
            ["on"] => {
                self.sim.enable_vcd();
                Ok(Response::line("dump on"))
            }
            ["off"] => {
                self.dump = self.sim.disable_vcd();
                Ok(Response::line("dump off"))
            }
            _ => Err(SessionError::Usage("dump on|off")),
        }
    }

    fn cmd_memory(&mut self, args: &[&str]) -> Result<Response, SessionError> {
        let joined = args.join("");
        if joined.is_empty() {
            return Err(SessionError::Usage("memory <name>[<index>]"));
        }
        let (name, index) = match joined.split_once('[') {
            Some((name, rest)) => (
                name,
                Some(
                    rest.strip_suffix(']')
                        .ok_or_else(|| SessionError::BadIndex(rest.to_owned()))?,
                ),
            ),
            None => (joined.as_str(), None),
        };
        let mem = self.resolve_memory(name)?;
        let full = self.memory_name(name);
        let len = self.sim.mem_len(mem);
        let (lo, hi) = match index {
            None => (0usize, len.min(MEMORY_PREVIEW)),
            Some(text) => match text.split_once(':') {
                Some((a, b)) => (parse_index(a)?, parse_index(b)?.saturating_add(1)),
                None => {
                    let i = parse_index(text)?;
                    (i, i.saturating_add(1))
                }
            },
        };
        let mut lines = Vec::new();
        if index.is_none() {
            lines.push(format!("{full}: {len} elements"));
        }
        for i in lo..hi.min(len) {
            let value = self
                .sim
                .get_mem(mem, u64::try_from(i).unwrap_or(u64::MAX))
                .map_or_else(|| "?".to_owned(), |v| v.to_string());
            lines.push(format!("  [{i}] = {value}"));
        }
        if lo >= len {
            lines.push(format!("  [{lo}] is out of range"));
        }
        Ok(Response { lines, quit: false })
    }

    fn cmd_scope(&mut self, args: &[&str]) -> Result<Response, SessionError> {
        match args {
            [] => Ok(Response::line(format!("scope = {}", self.scope))),
            [path] => {
                let path = if self.sim.instance_module(path).is_some() {
                    (*path).to_owned()
                } else {
                    let qualified = format!("{}.{path}", self.scope);
                    if self.sim.instance_module(&qualified).is_some() {
                        qualified
                    } else {
                        return Err(SessionError::UnknownScope((*path).to_owned()));
                    }
                };
                self.scope = path;
                Ok(Response::line(format!("scope = {}", self.scope)))
            }
            _ => Err(SessionError::Usage("scope [path]")),
        }
    }

    fn cmd_where(&mut self) -> Result<Response, SessionError> {
        let module = self.sim.instance_module(&self.scope).unwrap_or("?");
        Ok(Response::line(format!(
            "{} (module {module}) at time {}, {}",
            self.scope,
            self.sim.time(),
            status_word(self.sim.status())
        )))
    }

    // ---- helpers ----

    /// The breakpoint reports and the status line after a run.
    fn status_lines(&mut self, stepped: bool) -> Vec<String> {
        let mut lines = Vec::new();
        let hits = self.sim.take_breakpoint_hits();
        let stopped = !hits.is_empty();
        for id in hits {
            match self.sim.breakpoint(id) {
                Some((net, value)) => {
                    let name = self.sim.net_name(net).to_owned();
                    lines.push(match value {
                        Some(v) => format!("breakpoint {} on {name} = {v}", id.number()),
                        None => format!("breakpoint {} on {name}", id.number()),
                    });
                }
                None => lines.push(format!("breakpoint {}", id.number())),
            }
        }
        let time = self.sim.time();
        lines.push(match self.sim.status() {
            Status::Finished => format!("finished at time {time}"),
            Status::Stopped => format!("stopped at time {time}"),
            Status::Running if stopped => format!("stopped at time {time}"),
            Status::Running if stepped => format!("stepped to time {time}"),
            Status::Running => format!("ran to time {time}"),
        });
        lines
    }

    /// Resolves a net name in the scope, then at the top.
    fn resolve(&self, name: &str) -> Result<NetHandle, SessionError> {
        self.sim
            .net(name)
            .or_else(|| self.sim.net(&format!("{}.{name}", self.scope)))
            .ok_or_else(|| SessionError::UnknownNet(name.to_owned()))
    }

    fn resolve_memory(&self, name: &str) -> Result<MemHandle, SessionError> {
        self.sim
            .memory(name)
            .or_else(|| self.sim.memory(&format!("{}.{name}", self.scope)))
            .ok_or_else(|| SessionError::UnknownMemory(name.to_owned()))
    }

    /// The name a memory is shown under.
    fn memory_name(&self, name: &str) -> String {
        if self.sim.memory(name).is_some() {
            name.to_owned()
        } else {
            format!("{}.{name}", self.scope)
        }
    }

    /// Parses a value literal.
    fn value(&self, text: &str) -> Result<Logic, SessionError> {
        Logic::parse_verilog(text).map_err(|_| SessionError::BadValue(text.to_owned()))
    }

    /// Parses a time: digits with an optional unit suffix.
    fn ticks(&self, text: &str) -> Result<u64, SessionError> {
        let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return Err(SessionError::BadTime(text.to_owned()));
        }
        let value: u64 = digits
            .parse()
            .map_err(|_| SessionError::BadTime(text.to_owned()))?;
        let unit = text[digits.len()..].trim();
        if unit.is_empty() {
            return Ok(value);
        }
        let unit =
            TimeUnit::from_name(unit).ok_or_else(|| SessionError::BadTime(text.to_owned()))?;
        Ok(self.sim.ticks(Delay::new(value, unit)))
    }

    /// The completion candidates for a partly typed command line.
    ///
    /// With no whitespace yet the candidates are command names; after a
    /// command that takes a net, they are net paths, both hierarchical and
    /// relative to the scope. The result is sorted and deduplicated.
    pub fn complete(&self, line: &str) -> Vec<String> {
        let line = line.trim_start();
        let Some((command, rest)) = line.split_once(char::is_whitespace) else {
            return COMMANDS
                .iter()
                .filter(|c| c.starts_with(line))
                .map(|c| (*c).to_owned())
                .collect();
        };
        let word = if rest.is_empty() || rest.ends_with(char::is_whitespace) {
            ""
        } else {
            rest.split_whitespace().last().unwrap_or("")
        };
        let mut out: Vec<String> = match command {
            "dump" => ["on", "off"]
                .iter()
                .filter(|c| c.starts_with(word))
                .map(|c| (*c).to_owned())
                .collect(),
            "scope" => self
                .sim
                .instance_paths()
                .into_iter()
                .flat_map(|path| self.candidates(path, word))
                .collect(),
            "break" | "b" | "force" | "release" | "print" | "p" | "watch" => self
                .sim
                .nets()
                .into_iter()
                .flat_map(|(path, _)| self.candidates(path, word))
                .collect(),
            "memory" | "mem" => self
                .sim
                .memories()
                .into_iter()
                .flat_map(|(path, _)| self.candidates(path, word))
                .collect(),
            _ => Vec::new(),
        };
        out.sort();
        out.dedup();
        out
    }

    /// The spellings of `path` that start with `word`: the full path, and
    /// the tail after the current scope.
    fn candidates(&self, path: String, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        if path.starts_with(word) {
            out.push(path.clone());
        }
        if let Some(rel) = path.strip_prefix(&format!("{}.", self.scope))
            && rel.starts_with(word)
        {
            out.push(rel.to_owned());
        }
        out
    }
}

/// One word for a run state.
fn status_word(status: Status) -> &'static str {
    match status {
        Status::Running => "running",
        Status::Stopped => "stopped",
        Status::Finished => "finished",
    }
}

/// Parses a memory index.
fn parse_index(text: &str) -> Result<usize, SessionError> {
    text.trim()
        .parse()
        .map_err(|_| SessionError::BadIndex(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Design;
    use crate::sim::{SimOptions, Simulator};
    use crate::source::SourceMap;

    const DESIGN: &str = "\
top tb

module tb
  timescale 1 ns / 1 ps
  net %clk u1 reg
  net %rst u1 reg
  net %q u4 reg
  memory @ram 4 x u8
  instance u_sub of sub (a=%q)
  process clkgen free
    wait for 8'd5
    %clk = not(%clk)
  end
  process stim initial
    %clk = 1'd0
    %rst = 1'd1
    %q = 4'd0
    memwrite @ram[4'd1] = 8'd7
    wait for 8'd12
    %rst = 1'd0
    wait for 8'd60
    finish
  end
  process count seq posedge %clk
    if %rst
      %q <= 4'd0
    else
      %q <= add(%q, 4'd1)
    end
  end
end

module sub
  net %a u4 wire
  net %b u4 wire
  port a in %a
  assign %b = %a
end
";

    fn session(design: &Design) -> Session<'_> {
        let sim = Simulator::new(design, SimOptions::default()).expect("elaborates");
        Session::new(sim)
    }

    fn design() -> Design {
        let mut map = SourceMap::new();
        let file = map.add("tb.rtl", DESIGN).unwrap();
        Design::parse_text(DESIGN, file).expect("parses")
    }

    /// Runs a command and returns its response text, panicking on error.
    fn ok(s: &mut Session<'_>, line: &str) -> String {
        s.execute(line)
            .unwrap_or_else(|e| panic!("`{line}`: {e}"))
            .text()
    }

    #[test]
    fn empty_and_comments_do_nothing() {
        let d = design();
        let mut s = session(&d);
        assert_eq!(ok(&mut s, ""), "");
        assert_eq!(ok(&mut s, "   "), "");
        assert_eq!(ok(&mut s, "# a comment"), "");
    }

    #[test]
    fn time_run_and_step() {
        let d = design();
        let mut s = session(&d);
        assert_eq!(ok(&mut s, "time"), "time = 0\n");
        assert_eq!(ok(&mut s, "run 10ns"), "ran to time 10000\n");
        assert_eq!(ok(&mut s, "time"), "time = 10000\n");
        assert!(ok(&mut s, "step").starts_with("stepped to time "));
        assert_eq!(ok(&mut s, "run"), "finished at time 72000\n");
        assert_eq!(
            ok(&mut s, "where"),
            "tb (module tb) at time 72000, finished\n"
        );
        assert!(matches!(
            s.execute("run 1 2"),
            Err(SessionError::Usage("run [time]"))
        ));
        assert!(matches!(
            s.execute("run soon"),
            Err(SessionError::BadTime(_))
        ));
    }

    #[test]
    fn print_force_and_release() {
        let d = design();
        let mut s = session(&d);
        ok(&mut s, "run 1ns");
        assert_eq!(ok(&mut s, "print q"), "tb.q = 4'h0\n");
        assert_eq!(ok(&mut s, "print tb.q"), "tb.q = 4'h0\n");
        assert_eq!(ok(&mut s, "force q 4'h5"), "forced tb.q = 4'h5\n");
        assert_eq!(ok(&mut s, "print q"), "tb.q = 4'h5\n");
        assert_eq!(ok(&mut s, "force q = 4'h6"), "forced tb.q = 4'h6\n");
        assert_eq!(ok(&mut s, "release q"), "released tb.q\n");
        assert!(matches!(
            s.execute("print nothing"),
            Err(SessionError::UnknownNet(_))
        ));
        assert!(matches!(
            s.execute("force q zzz"),
            Err(SessionError::BadValue(_))
        ));
        assert!(matches!(
            s.execute("force q"),
            Err(SessionError::Usage("force <net> <value>"))
        ));
    }

    #[test]
    fn breakpoints_stop_the_run() {
        let d = design();
        let mut s = session(&d);
        assert_eq!(ok(&mut s, "break tb.q"), "breakpoint 0 on tb.q\n");
        let text = ok(&mut s, "run");
        assert!(text.contains("breakpoint 0 on tb.q"), "{text}");
        assert!(text.contains("stopped at time "), "{text}");
        // The run really stopped early: `$finish` is at 72 ns.
        assert!(s.simulator().time() < 72_000);
        assert_eq!(ok(&mut s, "delete 0"), "deleted breakpoint 0\n");
        assert!(matches!(
            s.execute("delete 0"),
            Err(SessionError::UnknownBreakpoint(0))
        ));
        assert_eq!(ok(&mut s, "run"), "finished at time 72000\n");
    }

    #[test]
    fn breakpoint_on_a_value() {
        let d = design();
        let mut s = session(&d);
        assert_eq!(
            ok(&mut s, "break tb.q 4'h3"),
            "breakpoint 0 on tb.q = 4'h3\n"
        );
        let text = ok(&mut s, "continue");
        assert!(text.contains("breakpoint 0 on tb.q = 4'h3"), "{text}");
        assert_eq!(ok(&mut s, "print tb.q"), "tb.q = 4'h3\n");
        assert_eq!(ok(&mut s, "delete"), "deleted 1 breakpoints\n");
    }

    #[test]
    fn watches_report_changes() {
        let d = design();
        let mut s = session(&d);
        assert_eq!(ok(&mut s, "watch tb.clk"), "watching tb.clk\n");
        assert_eq!(ok(&mut s, "watch clk"), "already watching tb.clk\n");
        let text = ok(&mut s, "run 12ns");
        assert!(text.contains("5000: tb.clk = 1'h1"), "{text}");
        assert!(text.contains("10000: tb.clk = 1'h0"), "{text}");
        assert!(text.ends_with("ran to time 12000\n"), "{text}");
    }

    #[test]
    fn memory_display() {
        let d = design();
        let mut s = session(&d);
        ok(&mut s, "run 1ns");
        let text = ok(&mut s, "memory ram");
        assert!(text.starts_with("tb.ram: 4 elements\n"), "{text}");
        assert!(text.contains("[1] = 8'h07"), "{text}");
        assert_eq!(ok(&mut s, "memory ram[1]"), "  [1] = 8'h07\n");
        let text = ok(&mut s, "memory tb.ram[0:1]");
        assert_eq!(text.lines().count(), 2);
        assert!(ok(&mut s, "memory ram[9]").contains("out of range"));
        assert!(matches!(
            s.execute("memory nope"),
            Err(SessionError::UnknownMemory(_))
        ));
        assert!(matches!(
            s.execute("memory ram[x]"),
            Err(SessionError::BadIndex(_))
        ));
    }

    #[test]
    fn scope_changes_name_resolution() {
        let d = design();
        let mut s = session(&d);
        assert_eq!(ok(&mut s, "scope"), "scope = tb\n");
        assert_eq!(ok(&mut s, "scope tb.u_sub"), "scope = tb.u_sub\n");
        assert!(ok(&mut s, "print b").starts_with("tb.u_sub.b = "));
        assert_eq!(
            ok(&mut s, "where"),
            "tb.u_sub (module sub) at time 0, running\n"
        );
        assert_eq!(ok(&mut s, "scope tb"), "scope = tb\n");
        // Relative scope names work too.
        assert_eq!(ok(&mut s, "scope u_sub"), "scope = tb.u_sub\n");
        assert!(matches!(
            s.execute("scope nowhere"),
            Err(SessionError::UnknownScope(_))
        ));
    }

    #[test]
    fn dump_captures_a_waveform() {
        let d = design();
        let mut s = session(&d);
        assert_eq!(ok(&mut s, "dump on"), "dump on\n");
        ok(&mut s, "run 20ns");
        assert_eq!(ok(&mut s, "dump off"), "dump off\n");
        let vcd = s.dump().expect("captured");
        assert!(vcd.contains("$enddefinitions"), "{vcd}");
        assert!(vcd.contains("tb"), "{vcd}");
        assert!(matches!(
            s.execute("dump sideways"),
            Err(SessionError::Usage("dump on|off"))
        ));
    }

    #[test]
    fn help_and_quit() {
        let d = design();
        let mut s = session(&d);
        let text = ok(&mut s, "help");
        for command in COMMANDS {
            assert!(text.contains(command), "`{command}` missing from help");
        }
        let response = s.execute("quit").expect("quit");
        assert!(response.quit);
        assert_eq!(response.text(), "quit\n");
        assert!(matches!(
            s.execute("frobnicate"),
            Err(SessionError::UnknownCommand(_))
        ));
        assert_eq!(
            SessionError::UnknownNet("x".into()).to_string(),
            "unknown net `x`"
        );
    }

    #[test]
    fn completion_candidates() {
        let d = design();
        let s = session(&d);
        assert_eq!(s.complete("pr"), vec!["print".to_owned()]);
        assert!(s.complete("").len() == COMMANDS.len());
        let nets = s.complete("print tb.");
        assert!(nets.contains(&"tb.clk".to_owned()), "{nets:?}");
        assert!(nets.contains(&"tb.u_sub.a".to_owned()), "{nets:?}");
        // Relative spellings are offered inside the scope.
        let nets = s.complete("print q");
        assert_eq!(nets, vec!["q".to_owned()]);
        assert_eq!(
            s.complete("dump o"),
            vec!["off".to_owned(), "on".to_owned()]
        );
        let scopes = s.complete("scope ");
        assert!(scopes.contains(&"tb.u_sub".to_owned()), "{scopes:?}");
        assert!(s.complete("memory ra").contains(&"ram".to_owned()));
        assert!(s.complete("time ").is_empty());
    }

    #[test]
    fn response_text_formatting() {
        let response = Response::default();
        assert_eq!(response.text(), "");
        let response = Response::line("hello");
        assert_eq!(response.text(), "hello\n");
        let d = design();
        let s = session(&d);
        assert!(format!("{s:?}").contains("Session"));
    }
}
