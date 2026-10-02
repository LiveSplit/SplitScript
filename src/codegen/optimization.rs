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
