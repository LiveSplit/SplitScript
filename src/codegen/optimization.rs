//! Release-only size optimization. Debug bypasses this entire pipeline.
use super::{CodegenReport, merging, peephole};

pub(super) fn optimize(mut wasm: Vec<u8>, report: Option<&mut CodegenReport>) -> Vec<u8> {
    // A second sweep simplifies patterns exposed by shared returns and control
    // cleanup. Keep compilation bounded rather than iterating to a fixed point.
    for _ in 0..2 {
        let candidate = peephole::optimize(
            &wasm,
            peephole::Passes {
                instructions: true,
                constants: true,
                dead_code: true,
                locals: true,
                control: true,
                returns: true,
            },
        );
        if candidate.len() >= wasm.len() {
            break;
        }
        wasm = candidate;
    }
    merging::optimize(&wasm, report)
}
