//! The unified design IR.
//!
//! Both frontends lower into the types defined here, and every later stage
//! (simulation, synthesis, formal, emission) consumes them. See phase 3 of
//! `ROADMAP.md` for the intended shape: a hierarchical `Design` of modules,
//! each holding ports, nets, instances, a *process* form (structured
//! statements with sensitivity, for simulation and synthesis lowering) and a
//! *cell* form (primitive combinational and sequential cells, for after
//! synthesis), with attributes and source spans on every object.
//!
//! Nothing is defined yet; the module exists so the crate layout matches the
//! roadmap from the first commit.
