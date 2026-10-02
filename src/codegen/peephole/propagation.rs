//! Propagate integer locals whose explicit writes all assign the same literal.
//! Structured scopes establish dominance without a control-flow fixed point.
use super::{I, fold, simplify};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Constant {
    I32(i32),
    I64(i64),
}

impl Constant {
    fn from_op(op: &I<'_>) -> Option<Self> {
        match op {
            I::I32Const(value) => Some(Self::I32(*value)),
            I::I64Const(value) => Some(Self::I64(*value)),
            _ => None,
        }
    }

    fn instruction(self) -> I<'static> {
        match self {
            Self::I32(value) => I::I32Const(value),
            Self::I64(value) => I::I64Const(value),
        }
    }
}

pub(super) fn fold_locals(ops: &mut Vec<I<'_>>) {
    if ops
        .iter_mut()
        .any(|op| super::control::map_labels(op, |depth| depth).is_err())
    {
        return;
    }
    // None means no write; Some(None) permanently disqualifies a local.
    let mut values: Vec<Option<Option<Constant>>> = Vec::new();
    for (position, op) in ops.iter().enumerate() {
        if let I::LocalSet(index) | I::LocalTee(index) = op {
            let index = *index as usize;
            values.resize(values.len().max(index + 1), None);
            let value = position
                .checked_sub(1)
                .and_then(|i| Constant::from_op(&ops[i]));
            values[index] = Some(match values[index] {
                None => value,
                Some(previous) if previous == value => value,
                _ => None,
            });
        }
    }
    if !values.iter().any(|value| matches!(value, Some(Some(_)))) {
        return;
    }
    let mut known = vec![false; values.len()];
    // A fact starts only after a store, including for parameters and zero-
    // initialized locals. Discard facts introduced inside a scope at its exit
    // or else arm: a branch may bypass those stores. Facts from outer scopes
    // survive loops and calls because every possible write has the same value.
    let mut activated = Vec::new();
    let mut scopes = Vec::new();
    let mut result = Vec::with_capacity(ops.len());
    for mut op in ops.drain(..) {
        match op {
            I::Block(_) | I::Loop(_) | I::If(_) => scopes.push(activated.len()),
            I::Else | I::End => {
                if let Some(&start) = scopes.last() {
                    for index in activated.drain(start..) {
                        known[index] = false;
                    }
                    if matches!(op, I::End) {
                        scopes.pop();
                    }
                }
            }
            I::LocalSet(index) | I::LocalTee(index) => {
                let index = index as usize;
                if matches!(values[index], Some(Some(_))) && !known[index] {
                    known[index] = true;
                    activated.push(index);
                }
            }
            I::LocalGet(index) => {
                let index = index as usize;
                if known.get(index) == Some(&true) {
                    op = values[index].flatten().unwrap().instruction();
                }
            }
            _ => {}
        }
        result.push(op);
        loop {
            let before = result.len();
            fold(&mut result);
            simplify(&mut result);
            if before == result.len() {
                break;
            }
        }
    }
    *ops = result;
}
