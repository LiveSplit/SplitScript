//! Release-only size optimization. Debug bypasses this entire pipeline.
use super::{CodegenReport, global_cleanup, merging, peephole};

pub(super) fn cleanup(
    mut wasm: Vec<u8>,
    report: Option<&mut CodegenReport>,
    flow_locals: bool,
) -> Vec<u8> {
    // Repeat while bodies shrink: branch and local cleanup expose each other.
    // Most modules stop early; cap the sweeps for unusually large scripts.
    for _ in 0..6 {
        wasm = global_cleanup::optimize(&wasm);
        let candidate = peephole::optimize(
            &wasm,
            peephole::Passes {
                instructions: true,
                constants: true,
                dead_code: true,
                locals: true,
                control: true,
                returns: true,
                propagation: true,
                flow_locals,
                expressions: false,
                discarded_values: false,
            },
        );
        if candidate.len() >= wasm.len() {
            break;
        }
        wasm = candidate;
    }
    merging::optimize(&wasm, report)
}

pub(super) fn optimize(wasm: Vec<u8>, mut report: Option<&mut CodegenReport>) -> Vec<u8> {
    let baseline = cleanup(wasm, report.as_deref_mut(), false);
    let mut specialized = baseline.clone();
    let mut specialized_report = report.as_deref().cloned();
    // A changed signature can expose constants in its callers. Limit retries,
    // and stop as soon as a round cannot reduce the complete module.
    for _ in 0..3 {
        let candidate = super::arguments::specialize(&specialized);
        if candidate == specialized {
            break;
        }
        let mut candidate_report = specialized_report.clone();
        let candidate = cleanup(candidate, candidate_report.as_mut(), false);
        let candidate = super::type_pruning::optimize(&candidate);
        if candidate.len() >= specialized.len() {
            break;
        }
        specialized = candidate;
        specialized_report = candidate_report;
    }
    let mut plain_report = report.as_deref().cloned();
    let plain = finish_inlining(baseline.clone(), plain_report.as_mut());
    // Early specialization can interfere with later inlining and sharing.
    // Compare both complete pipelines instead of relying on an interim saving.
    let (result, selected_report) = if specialized.len() < baseline.len() {
        let candidate = finish_inlining(specialized, specialized_report.as_mut());
        if candidate.len() < plain.len() {
            (candidate, specialized_report)
        } else {
            (plain, plain_report)
        }
    } else {
        (plain, plain_report)
    };
    if let Some(report) = report {
        *report = selected_report.unwrap();
    }
    let mut result = peephole::optimize(
        &result,
        peephole::Passes {
            expressions: true,
            locals: true,
            ..Default::default()
        },
    );
    // Reused expressions introduce temporaries and expose dead calculations.
    // Revisit cleanup, retaining only strict reductions in the complete file.
    for _ in 0..6 {
        let candidate = peephole::optimize(
            &result,
            peephole::Passes {
                instructions: true,
                constants: true,
                locals: true,
                flow_locals: true,
                propagation: true,
                control: true,
                returns: true,
                dead_code: true,
                discarded_values: true,
                ..Default::default()
            },
        );
        if candidate.len() >= result.len() {
            break;
        }
        result = candidate;
    }
    result
}

fn finish_inlining(baseline: Vec<u8>, report: Option<&mut CodegenReport>) -> Vec<u8> {
    let (expanded, indices) = super::inlining::expand(&baseline);
    let mut candidate_report = report.as_deref().cloned();
    if let Some(report) = &mut candidate_report {
        report.functions.retain_mut(|(index, name)| {
            if let Some(mapped) = indices[*index as usize] {
                *index = mapped;
                true
            } else {
                report.inlined_functions.push(name.clone());
                false
            }
        });
    }
    let candidate = cleanup(expanded, candidate_report.as_mut(), true);
    let candidate = super::type_pruning::optimize(&candidate);
    if candidate.len() < baseline.len() {
        if let Some(report) = report {
            *report = candidate_report.unwrap();
        }
        candidate
    } else {
        baseline
    }
}
