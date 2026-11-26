//! Internal symbolic verifier for driver models (Phase 8, Layer 4).
//!
//! A candidate driver declares a one-tick **affine model** of the quantity it
//! changes: `next = (num * cur + offset) / den`. A [`Verified`] token asserts
//! that every real `cur` in a premise interval maps into a conclusion interval.
//! The token has a private field, so only [`verify_affine`] can create one.
//!
//! The decision procedure is internal and exact:
//! - [`Rat`] is a normalised rational on checked `i128` arithmetic.
//! - [`unsat`] runs Fourier-Motzkin elimination over linear constraints. Strict
//!   and non-strict inequalities are handled exactly, so the answer matches the
//!   reals, which is the right model for `f32` driver state.
//! - [`entails`] proves a conclusion by refuting its negation together with the
//!   premises.
//!
//! The procedure fails closed. An overflowing intermediate value, or a constraint
//! set past [`MAX_CONSTRAINTS`], gives "undecided", and verification fails.
//!
//! Limits: the model abstracts the driver. Verification proves the declared model
//! maps the invariant into itself. It does not prove the driver's code matches the
//! model. The repair loop closes that gap with a shadow check on the checkpoint
//! world (see `repair.rs`).

use std::collections::BTreeMap;

/// Upper bound on constraints kept during elimination. Beyond it the procedure
/// reports "undecided" rather than exhausting memory.
pub const MAX_CONSTRAINTS: usize = 20_000;

/// A normalised rational `num / den`, with `den > 0` and `gcd(|num|, den) = 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rat {
    num: i128,
    den: i128,
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

// `add`/`mul`/`neg` return `Option` (checked arithmetic), so they are not the std
// operator traits; the lint does not apply.
#[allow(clippy::should_implement_trait)]
impl Rat {
    pub const ZERO: Rat = Rat { num: 0, den: 1 };

    /// Build `num / den`. Returns `None` for a zero denominator or overflow.
    pub fn new(num: i128, den: i128) -> Option<Rat> {
        if den == 0 {
            return None;
        }
        let g = gcd(num, den).max(1);
        let (mut n, mut d) = (num / g, den / g);
        if d < 0 {
            n = n.checked_neg()?;
            d = d.checked_neg()?;
        }
        Some(Rat { num: n, den: d })
    }

    pub fn int(n: i64) -> Rat {
        Rat {
            num: n as i128,
            den: 1,
        }
    }

    pub fn num(self) -> i128 {
        self.num
    }

    pub fn den(self) -> i128 {
        self.den
    }

    pub fn is_zero(self) -> bool {
        self.num == 0
    }

    fn sign(self) -> i32 {
        self.num.signum() as i32
    }

    pub fn add(self, o: Rat) -> Option<Rat> {
        let n = self
            .num
            .checked_mul(o.den)?
            .checked_add(o.num.checked_mul(self.den)?)?;
        Rat::new(n, self.den.checked_mul(o.den)?)
    }

    pub fn mul(self, o: Rat) -> Option<Rat> {
        Rat::new(self.num.checked_mul(o.num)?, self.den.checked_mul(o.den)?)
    }

    pub fn neg(self) -> Option<Rat> {
        Some(Rat {
            num: self.num.checked_neg()?,
            den: self.den,
        })
    }
}

/// Identifies a variable in a constraint.
pub type VarId = usize;

/// The relation between a constraint's left-hand side and zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rel {
    /// `lhs <= 0`
    Le,
    /// `lhs < 0`
    Lt,
    /// `lhs = 0`
    Eq,
}

/// A linear constraint: `sum(coeff * var) + constant  rel  0`.
#[derive(Debug, Clone, PartialEq)]
pub struct Constraint {
    pub terms: Vec<(VarId, Rat)>,
    pub constant: Rat,
    pub rel: Rel,
}

/// Internal inequality form: `sum(coeff * var) + constant (<= or <) 0`.
#[derive(Debug, Clone)]
struct Ineq {
    terms: BTreeMap<VarId, Rat>,
    constant: Rat,
    strict: bool,
}

/// `ka * a + kb * b`, with variable terms cancelled when they reach zero.
fn combine(a: &Ineq, ka: Rat, b: &Ineq, kb: Rat) -> Option<Ineq> {
    let mut terms: BTreeMap<VarId, Rat> = BTreeMap::new();
    for (&v, &c) in &a.terms {
        terms.insert(v, c.mul(ka)?);
    }
    for (&v, &c) in &b.terms {
        let sum = terms
            .get(&v)
            .copied()
            .unwrap_or(Rat::ZERO)
            .add(c.mul(kb)?)?;
        if sum.is_zero() {
            terms.remove(&v);
        } else {
            terms.insert(v, sum);
        }
    }
    let constant = a.constant.mul(ka)?.add(b.constant.mul(kb)?)?;
    Some(Ineq {
        terms,
        constant,
        strict: a.strict || b.strict,
    })
}

fn to_ineqs(c: &Constraint) -> Option<Vec<Ineq>> {
    let mut terms: BTreeMap<VarId, Rat> = BTreeMap::new();
    for &(v, coeff) in &c.terms {
        let sum = terms.get(&v).copied().unwrap_or(Rat::ZERO).add(coeff)?;
        if sum.is_zero() {
            terms.remove(&v);
        } else {
            terms.insert(v, sum);
        }
    }
    let neg_terms = |t: &BTreeMap<VarId, Rat>| -> Option<BTreeMap<VarId, Rat>> {
        t.iter().map(|(&v, &k)| Some((v, k.neg()?))).collect()
    };
    let ineq = |terms: BTreeMap<VarId, Rat>, constant: Rat, strict: bool| Ineq {
        terms,
        constant,
        strict,
    };
    Some(match c.rel {
        Rel::Le => vec![ineq(terms, c.constant, false)],
        Rel::Lt => vec![ineq(terms, c.constant, true)],
        Rel::Eq => vec![
            ineq(terms.clone(), c.constant, false),
            ineq(neg_terms(&terms)?, c.constant.neg()?, false),
        ],
    })
}

/// `Some(true)` if `cs` has no real solution, `Some(false)` if it has one, and
/// `None` if the procedure could not decide (overflow or size cap).
pub fn unsat(cs: &[Constraint]) -> Option<bool> {
    let mut ineqs: Vec<Ineq> = Vec::new();
    for c in cs {
        ineqs.extend(to_ineqs(c)?);
    }
    loop {
        // Constant inequalities are decided as soon as they appear.
        let mut remaining = Vec::with_capacity(ineqs.len());
        for i in ineqs {
            if i.terms.is_empty() {
                let s = i.constant.sign();
                if s > 0 || (s == 0 && i.strict) {
                    return Some(true);
                }
            } else {
                remaining.push(i);
            }
        }
        ineqs = remaining;
        let Some(var) = ineqs.iter().find_map(|i| i.terms.keys().next().copied()) else {
            return Some(false);
        };

        let (mut pos, mut neg, mut rest) = (Vec::new(), Vec::new(), Vec::new());
        for i in ineqs {
            match i.terms.get(&var).map(|c| c.sign()) {
                Some(s) if s > 0 => pos.push(i),
                Some(_) => neg.push(i),
                None => rest.push(i),
            }
        }
        for p in &pos {
            for n in &neg {
                // p: a_p*x + R_p <= 0 (a_p > 0); n: a_n*x + R_n <= 0 (a_n < 0).
                // Scale p by -a_n > 0 and n by a_p > 0, then add: x cancels.
                let (a_p, a_n) = (p.terms[&var], n.terms[&var]);
                rest.push(combine(p, a_n.neg()?, n, a_p)?);
                if rest.len() > MAX_CONSTRAINTS {
                    return None;
                }
            }
        }
        ineqs = rest;
    }
}

/// Does `premises` entail `concl`? `None` if undecided.
pub fn entails(premises: &[Constraint], concl: &Constraint) -> Option<bool> {
    // The negation of `e <= 0` is `-e < 0`; the negation of `e < 0` is `-e <= 0`.
    // The negation of `e = 0` is `e < 0` or `-e < 0`, and both must be refuted.
    let negations: Vec<Constraint> = match concl.rel {
        Rel::Le => vec![negated(concl, Rel::Lt)?],
        Rel::Lt => vec![negated(concl, Rel::Le)?],
        Rel::Eq => vec![
            Constraint {
                rel: Rel::Lt,
                ..concl.clone()
            },
            negated(concl, Rel::Lt)?,
        ],
    };
    for neg in negations {
        let mut all = premises.to_vec();
        all.push(neg);
        if !unsat(&all)? {
            return Some(false);
        }
    }
    Some(true)
}

fn negated(c: &Constraint, rel: Rel) -> Option<Constraint> {
    let terms = c
        .terms
        .iter()
        .map(|&(v, k)| Some((v, k.neg()?)))
        .collect::<Option<Vec<_>>>()?;
    Some(Constraint {
        terms,
        constant: c.constant.neg()?,
        rel,
    })
}

/// `next = (num * cur + offset) / den`, with `den > 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AffineModel {
    pub num: i64,
    pub den: i64,
    pub offset: i64,
}

impl AffineModel {
    /// Build a model. Rejects a non-positive denominator.
    pub fn new(num: i64, den: i64, offset: i64) -> Result<Self, VerifyError> {
        if den <= 0 {
            return Err(VerifyError::InvalidModel);
        }
        Ok(AffineModel { num, den, offset })
    }

    /// The slope `num / den` as a rational.
    fn slope(&self) -> Option<Rat> {
        Rat::new(self.num as i128, self.den as i128)
    }

    /// The intercept `offset / den` as a rational.
    fn intercept(&self) -> Option<Rat> {
        Rat::new(self.offset as i128, self.den as i128)
    }

    /// Evaluate on an integer input. `None` if the result is not an integer, or
    /// on overflow.
    pub fn apply(&self, cur: i64) -> Option<i64> {
        let v = self.slope()?.mul(Rat::int(cur))?.add(self.intercept()?)?;
        if v.den() != 1 {
            return None;
        }
        i64::try_from(v.num()).ok()
    }
}

/// A closed integer interval `[lo, hi]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub lo: i64,
    pub hi: i64,
}

impl Bounds {
    pub fn new(lo: i64, hi: i64) -> Result<Self, VerifyError> {
        if lo > hi {
            return Err(VerifyError::InvalidBounds);
        }
        Ok(Bounds { lo, hi })
    }

    pub fn contains(&self, v: i64) -> bool {
        self.lo <= v && v <= self.hi
    }
}

/// Proof that `model` maps every `premise` value into `conclusion`.
/// Only [`verify_affine`] constructs one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verified {
    model: AffineModel,
    premise: Bounds,
    conclusion: Bounds,
    _seal: (),
}

impl Verified {
    pub fn model(&self) -> AffineModel {
        self.model
    }
    pub fn premise(&self) -> Bounds {
        self.premise
    }
    pub fn conclusion(&self) -> Bounds {
        self.conclusion
    }
}

/// Why a model could not be verified.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("affine model has a non-positive denominator")]
    InvalidModel,
    #[error("bounds have lo > hi")]
    InvalidBounds,
    #[error("the model does not map the premise into the conclusion")]
    NotEntailed,
    #[error("the verifier could not decide (arithmetic overflow or size limit)")]
    Undecided,
}

const CUR: VarId = 0;
const NEXT: VarId = 1;

/// Verify that `model` maps every real in `premise` into `conclusion`.
pub fn verify_affine(
    model: AffineModel,
    premise: Bounds,
    conclusion: Bounds,
) -> Result<Verified, VerifyError> {
    if model.den <= 0 {
        return Err(VerifyError::InvalidModel);
    }
    if premise.lo > premise.hi || conclusion.lo > conclusion.hi {
        return Err(VerifyError::InvalidBounds);
    }
    let premises = affine_premises(model, premise).ok_or(VerifyError::Undecided)?;

    // Conclusion `next >= lo` is `-next + lo <= 0`; `next <= hi` is `next - hi <= 0`.
    let lower = Constraint {
        terms: vec![(NEXT, Rat::int(-1))],
        constant: Rat::int(conclusion.lo),
        rel: Rel::Le,
    };
    let upper = Constraint {
        terms: vec![(NEXT, Rat::int(1))],
        constant: Rat::int(-conclusion.hi),
        rel: Rel::Le,
    };
    for concl in [lower, upper] {
        match entails(&premises, &concl) {
            Some(true) => {}
            Some(false) => return Err(VerifyError::NotEntailed),
            None => return Err(VerifyError::Undecided),
        }
    }
    Ok(Verified {
        model,
        premise,
        conclusion,
        _seal: (),
    })
}

/// Premises: `lo <= cur <= hi` and `next - slope*cur - intercept = 0`.
fn affine_premises(model: AffineModel, premise: Bounds) -> Option<Vec<Constraint>> {
    let lo = Constraint {
        terms: vec![(CUR, Rat::int(-1))],
        constant: Rat::int(premise.lo),
        rel: Rel::Le,
    };
    let hi = Constraint {
        terms: vec![(CUR, Rat::int(1))],
        constant: Rat::int(-premise.hi),
        rel: Rel::Le,
    };
    let eq = Constraint {
        terms: vec![(NEXT, Rat::int(1)), (CUR, model.slope()?.neg()?)],
        constant: model.intercept()?.neg()?,
        rel: Rel::Eq,
    };
    Some(vec![lo, hi, eq])
}

/// Independent oracle for tests: an affine map on a real interval attains its
/// extremes at the endpoints, so checking those two values decides entailment.
#[cfg(test)]
fn interval_entails(model: AffineModel, premise: Bounds, conclusion: Bounds) -> bool {
    let f = |x: i64| -> Option<Rat> { model.slope()?.mul(Rat::int(x))?.add(model.intercept()?) };
    let (Some(fl), Some(fh)) = (f(premise.lo), f(premise.hi)) else {
        return false;
    };
    // Compare by value: cross-multiply, both denominators are positive.
    let le = |a: Rat, b: Rat| a.num() * b.den() <= b.num() * a.den();
    let (mn, mx) = if le(fl, fh) { (fl, fh) } else { (fh, fl) };
    le(Rat::int(conclusion.lo), mn) && le(mx, Rat::int(conclusion.hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(lo: i64, hi: i64) -> Bounds {
        Bounds::new(lo, hi).unwrap()
    }

    fn var(v: VarId, c: i64, k: i64, rel: Rel) -> Constraint {
        Constraint {
            terms: vec![(v, Rat::int(c))],
            constant: Rat::int(k),
            rel,
        }
    }

    #[test]
    fn rationals_normalise_and_reject_zero_denominators() {
        assert_eq!(Rat::new(2, 4), Rat::new(1, 2));
        assert_eq!(Rat::new(1, -2), Rat::new(-1, 2));
        assert!(Rat::new(1, 0).is_none());
        assert_eq!(
            Rat::new(1, 2).unwrap().add(Rat::new(1, 3).unwrap()),
            Rat::new(5, 6)
        );
    }

    #[test]
    fn rational_overflow_is_reported_not_wrapped() {
        let big = Rat::new(i128::MAX / 2, 1).unwrap();
        assert!(big.mul(big).is_none());
    }

    #[test]
    fn strictness_is_respected() {
        // x < 1 and x >= 1 is unsatisfiable; x <= 1 and x >= 1 is satisfiable.
        let lt = var(0, 1, -1, Rel::Lt);
        let ge = var(0, -1, 1, Rel::Le);
        assert_eq!(unsat(&[lt, ge.clone()]), Some(true));
        let le = var(0, 1, -1, Rel::Le);
        assert_eq!(unsat(&[le, ge]), Some(false));
    }

    #[test]
    fn multi_variable_entailment() {
        // x >= 0, y >= 0, x + y <= 10 entails x <= 10, but not x <= 5.
        let premises = vec![
            var(0, -1, 0, Rel::Le),
            var(1, -1, 0, Rel::Le),
            Constraint {
                terms: vec![(0, Rat::int(1)), (1, Rat::int(1))],
                constant: Rat::int(-10),
                rel: Rel::Le,
            },
        ];
        assert_eq!(entails(&premises, &var(0, 1, -10, Rel::Le)), Some(true));
        assert_eq!(entails(&premises, &var(0, 1, -5, Rel::Le)), Some(false));
    }

    #[test]
    fn strict_premises_do_not_entail_their_closure_edge() {
        // x < 1 entails x <= 1 but not x < 1 from x <= 1.
        let strict = vec![var(0, 1, -1, Rel::Lt)];
        assert_eq!(entails(&strict, &var(0, 1, -1, Rel::Le)), Some(true));
        let closed = vec![var(0, 1, -1, Rel::Le)];
        assert_eq!(entails(&closed, &var(0, 1, -1, Rel::Lt)), Some(false));
    }

    #[test]
    fn equalities_entail_both_sides() {
        let premises = vec![var(0, 1, -2, Rel::Eq)]; // x = 2
        assert_eq!(entails(&premises, &var(0, -1, 2, Rel::Le)), Some(true)); // x >= 2
        assert_eq!(entails(&premises, &var(0, 1, -3, Rel::Le)), Some(true)); // x <= 3
        assert_eq!(entails(&premises, &var(0, 1, -1, Rel::Le)), Some(false)); // x <= 1 fails
    }

    #[test]
    fn damping_keeps_the_interval() {
        let m = AffineModel::new(9, 10, 0).unwrap();
        let v = verify_affine(m, b(-10, 10), b(-10, 10)).expect("verifies");
        assert_eq!(v.model(), m);
    }

    #[test]
    fn growth_is_rejected() {
        let m = AffineModel::new(3, 2, 0).unwrap();
        assert_eq!(
            verify_affine(m, b(-10, 10), b(-10, 10)),
            Err(VerifyError::NotEntailed)
        );
    }

    #[test]
    fn offsets_and_negative_scaling() {
        assert!(verify_affine(AffineModel::new(1, 1, 1).unwrap(), b(0, 4), b(1, 5)).is_ok());
        assert!(verify_affine(AffineModel::new(1, 1, 1).unwrap(), b(0, 4), b(0, 4)).is_err());
        assert!(verify_affine(AffineModel::new(-2, 1, 0).unwrap(), b(-3, 3), b(-6, 6)).is_ok());
        assert!(verify_affine(AffineModel::new(-2, 1, 0).unwrap(), b(-3, 3), b(-5, 6)).is_err());
    }

    #[test]
    fn fractional_intercepts_are_exact() {
        // next = (-4*cur - 1)/2 maps [-5, 5] to [-10.5, 9.5]: not inside [-10, 10].
        // The reals refute this, so the verifier must not accept it.
        let m = AffineModel::new(-4, 2, -1).unwrap();
        assert_eq!(
            verify_affine(m, b(-5, 5), b(-10, 10)),
            Err(VerifyError::NotEntailed)
        );
        assert!(verify_affine(m, b(-5, 5), b(-11, 10)).is_ok());
    }

    #[test]
    fn malformed_inputs_are_rejected() {
        assert_eq!(AffineModel::new(1, 0, 0), Err(VerifyError::InvalidModel));
        assert_eq!(Bounds::new(5, 1), Err(VerifyError::InvalidBounds));
        let m = AffineModel::new(1, 1, 0).unwrap();
        let bad = Bounds { lo: 5, hi: 1 };
        assert_eq!(
            verify_affine(m, bad, b(0, 1)),
            Err(VerifyError::InvalidBounds)
        );
    }

    #[test]
    fn apply_is_exact_or_none() {
        let m = AffineModel::new(9, 10, 0).unwrap();
        assert_eq!(m.apply(10), Some(9));
        assert_eq!(m.apply(5), None); // 4.5 is not an integer
    }

    #[test]
    fn fourier_motzkin_agrees_with_the_interval_oracle_on_a_grid() {
        for num in -4..=4 {
            for den in 1..=3 {
                for off in -3..=3 {
                    let m = AffineModel::new(num, den, off).unwrap();
                    for (lo, hi) in [(-5, 5), (0, 3), (-2, 2), (-10, 10)] {
                        for conc in [b(-10, 10), b(-6, 6), b(0, 4)] {
                            let fm = verify_affine(m, b(lo, hi), conc).is_ok();
                            let oracle = interval_entails(m, b(lo, hi), conc);
                            assert_eq!(fm, oracle, "{m:?} {lo}..{hi} -> {conc:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_huge_model_fails_closed_instead_of_wrapping() {
        // Slope i64::MAX pushes intermediate products past what i128 rationals hold
        // after composition; the result must be an error, never a wrong proof.
        let m = AffineModel::new(i64::MAX, 1, i64::MAX).unwrap();
        assert!(verify_affine(m, b(-10, 10), b(-10, 10)).is_err());
    }
}
