//! Lookup tables (`cell_rise`, `rise_transition`, `rise_constraint`,
//! `rise_power`, ...) and their templates.
//!
//! A Liberty table is a scalar, a one-dimensional or a two-dimensional grid
//! of values indexed by the template's variables (input transition time,
//! output load, related-pin transition, ...). Lookups between grid points
//! interpolate linearly along each axis and extrapolate linearly outside
//! the grid using the two nearest points, which is what the Liberty
//! reference prescribes and what OpenSTA does.

use super::syntax::Group;

/// A `lu_table_template` / `power_lut_template` group.
#[derive(Clone, Debug, PartialEq)]
pub struct LutTemplate {
    /// The template name, referenced by table groups.
    pub name: String,
    /// `variable_1`, `variable_2`, `variable_3` as written.
    pub variables: Vec<String>,
    /// Default `index_1` values.
    pub index_1: Vec<f64>,
    /// Default `index_2` values.
    pub index_2: Vec<f64>,
    /// Default `index_3` values.
    pub index_3: Vec<f64>,
}

impl LutTemplate {
    pub(super) fn from_group(g: &Group) -> LutTemplate {
        let mut variables = Vec::new();
        for key in ["variable_1", "variable_2", "variable_3"] {
            match g.attr_str(key) {
                Some(v) => variables.push(v.to_string()),
                None => break,
            }
        }
        LutTemplate {
            name: g.arg_name().unwrap_or_default().to_string(),
            variables,
            index_1: g.attr("index_1").map(|v| v.numbers()).unwrap_or_default(),
            index_2: g.attr("index_2").map(|v| v.numbers()).unwrap_or_default(),
            index_3: g.attr("index_3").map(|v| v.numbers()).unwrap_or_default(),
        }
    }
}

/// One lookup table: the values of a timing or power quantity over a grid.
///
/// `values` is stored row-major: the value at `index_1[i]`, `index_2[j]`
/// is `values[i * index_2.len() + j]`. A scalar table has one value and
/// empty indices; a one-dimensional table has `index_2` empty.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LutTable {
    /// The template name (`scalar` for constant tables).
    pub template: String,
    /// First axis, from the table or inherited from the template.
    pub index_1: Vec<f64>,
    /// Second axis, from the table or inherited from the template.
    pub index_2: Vec<f64>,
    /// Third axis when the template has three variables (not interpolated).
    pub index_3: Vec<f64>,
    /// The values, row-major over (`index_1`, `index_2`[, `index_3`]).
    pub values: Vec<f64>,
}

impl LutTable {
    /// Builds a table from its group, filling missing indices from the
    /// named template.
    pub(super) fn from_group(g: &Group, templates: &[LutTemplate]) -> LutTable {
        let template = g.arg_name().unwrap_or("scalar").to_string();
        let tmpl = templates.iter().find(|t| t.name == template);
        let index = |key: &str, from_template: fn(&LutTemplate) -> &Vec<f64>| -> Vec<f64> {
            match g.attr(key) {
                Some(v) => v.numbers(),
                None => tmpl.map(|t| from_template(t).clone()).unwrap_or_default(),
            }
        };
        LutTable {
            index_1: index("index_1", |t| &t.index_1),
            index_2: index("index_2", |t| &t.index_2),
            index_3: index("index_3", |t| &t.index_3),
            values: g.attr("values").map(|v| v.numbers()).unwrap_or_default(),
            template,
        }
    }

    /// A constant table.
    pub fn scalar(value: f64) -> LutTable {
        LutTable {
            template: "scalar".into(),
            values: vec![value],
            ..LutTable::default()
        }
    }

    /// Number of axes with more than one point (0, 1 or 2; 3 for tables
    /// that carry `index_3`).
    pub fn dimensions(&self) -> usize {
        if !self.index_3.is_empty() {
            3
        } else if !self.index_2.is_empty() {
            2
        } else if !self.index_1.is_empty() {
            1
        } else {
            0
        }
    }

    /// True when the value count matches the index sizes.
    pub fn is_consistent(&self) -> bool {
        let expected = match self.dimensions() {
            0 => 1,
            1 => self.index_1.len(),
            2 => self.index_1.len() * self.index_2.len(),
            _ => self.index_1.len() * self.index_2.len() * self.index_3.len(),
        };
        self.values.len() == expected
    }

    /// The value at `x` on the first axis and `y` on the second, with
    /// linear interpolation inside the grid and linear extrapolation
    /// outside it. `y` is ignored for one-dimensional tables and both are
    /// ignored for scalars. `None` for an empty, inconsistent or
    /// three-dimensional table.
    pub fn lookup(&self, x: f64, y: f64) -> Option<f64> {
        if self.values.is_empty() || !self.is_consistent() {
            return None;
        }
        match self.dimensions() {
            0 => Some(self.values[0]),
            1 => {
                let (i, t) = axis_position(&self.index_1, x);
                Some(interp(|k| self.values[k], i, t))
            }
            2 => {
                let cols = self.index_2.len();
                let (i, tx) = axis_position(&self.index_1, x);
                let (j, ty) = axis_position(&self.index_2, y);
                let at = |r: usize, c: usize| self.values[r * cols + c];
                let row = |r: usize| interp(|c| at(r, c), j, ty);
                Some(interp(row, i, tx))
            }
            _ => None,
        }
    }

    /// The largest value in the table, useful as a pessimistic bound.
    pub fn max_value(&self) -> Option<f64> {
        self.values.iter().copied().reduce(f64::max)
    }
}

/// Finds the segment of `axis` that `q` falls in (or the nearest segment
/// when outside), returning its first index and the parameter `t` along
/// it, where `t` may be outside `[0, 1]` for extrapolation. With a single
/// point the segment degenerates to that point (`t = 0`), and [`interp`]
/// then never reads index `i + 1`.
fn axis_position(axis: &[f64], q: f64) -> (usize, f64) {
    if axis.len() < 2 {
        return (0, 0.0);
    }
    let last = axis.len() - 1;
    let i = if q <= axis[0] {
        0
    } else if q >= axis[last] {
        last - 1
    } else {
        axis.iter()
            .rposition(|&a| a <= q)
            .unwrap_or(0)
            .min(last - 1)
    };
    let span = axis[i + 1] - axis[i];
    let t = if span == 0.0 {
        0.0
    } else {
        (q - axis[i]) / span
    };
    (i, t)
}

/// Linear interpolation between `get(i)` and `get(i + 1)` at parameter
/// `t`; `get(i + 1)` is only read when `t != 0`, which is what makes
/// single-point axes safe.
fn interp(get: impl Fn(usize) -> f64, i: usize, t: f64) -> f64 {
    let a = get(i);
    if t == 0.0 {
        a
    } else {
        let b = get(i + 1);
        a + (b - a) * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table2() -> LutTable {
        LutTable {
            template: "t".into(),
            index_1: vec![0.0, 1.0, 2.0],
            index_2: vec![0.0, 10.0],
            index_3: vec![],
            // rows: x = 0, 1, 2; cols: y = 0, 10
            values: vec![0.0, 10.0, 1.0, 11.0, 2.0, 12.0],
        }
    }

    #[test]
    fn scalar_and_one_dimensional() {
        assert_eq!(LutTable::scalar(3.5).lookup(9.0, 9.0), Some(3.5));
        let t = LutTable {
            template: "t".into(),
            index_1: vec![1.0, 2.0, 4.0],
            values: vec![10.0, 20.0, 40.0],
            ..LutTable::default()
        };
        assert_eq!(t.dimensions(), 1);
        assert_eq!(t.lookup(1.0, 0.0), Some(10.0));
        assert_eq!(t.lookup(1.5, 0.0), Some(15.0));
        assert_eq!(t.lookup(3.0, 0.0), Some(30.0));
        assert_eq!(t.lookup(0.0, 0.0), Some(0.0), "extrapolates below");
        assert_eq!(t.lookup(8.0, 0.0), Some(80.0), "extrapolates above");
    }

    #[test]
    fn bilinear_interpolation_and_extrapolation() {
        let t = table2();
        assert!(t.is_consistent());
        assert_eq!(t.lookup(1.0, 10.0), Some(11.0));
        assert_eq!(t.lookup(0.5, 5.0), Some(5.5));
        assert_eq!(t.lookup(1.5, 2.5), Some(4.0));
        // Extrapolation on both axes.
        assert_eq!(t.lookup(3.0, 20.0), Some(23.0));
        assert_eq!(t.lookup(-1.0, -10.0), Some(-11.0));
    }

    #[test]
    fn single_point_axes_and_bad_tables() {
        let t = LutTable {
            template: "t".into(),
            index_1: vec![0.5],
            index_2: vec![1.0, 2.0],
            values: vec![3.0, 4.0],
            ..LutTable::default()
        };
        assert_eq!(t.lookup(100.0, 1.5), Some(3.5));
        let mut bad = table2();
        bad.values.pop();
        assert!(!bad.is_consistent());
        assert_eq!(bad.lookup(0.0, 0.0), None);
        assert_eq!(LutTable::default().lookup(0.0, 0.0), None);
    }
}
