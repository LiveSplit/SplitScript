//! Remove branches whose destination is the immediately following fallthrough.
//! Stack heights must already satisfy every closing frame; branches can otherwise
//! discard extra operands that ordinary block/function ends would reject.
use wasmparser::{
    BlockType, CompositeInnerType, ContType, FrameKind, FuncType, ModuleArity, Operator as O,
    RefType, SubType,
};

#[derive(Default)]
pub(super) struct Types {
    pub types: Vec<SubType>,
    pub functions: Vec<u32>,
    pub imported: usize,
    pub globals: Vec<wasmparser::GlobalType>,
    pub shared: bool,
}

struct Frame {
    ty: BlockType,
    kind: FrameKind,
    base: u32,
    dead: bool,
}
struct Stack<'a> {
    types: &'a Types,
    frames: Vec<Frame>,
}

impl Stack<'_> {
    fn results(&self, ty: &BlockType) -> Option<&[wasmparser::ValType]> {
        match ty {
            BlockType::Empty => Some(&[]),
            // Use equality below rather than attempting GC subtyping inference.
            BlockType::Type(_) => None,
            BlockType::FuncType(index) => {
                match &self.types.types.get(*index as usize)?.composite_type.inner {
                    CompositeInnerType::Func(f) => Some(f.results()),
                    _ => None,
                }
            }
        }
    }
    fn same_results(&self, a: &BlockType, b: &BlockType) -> bool {
        match (a, b) {
            (BlockType::Type(a), BlockType::Type(b)) => a == b,
            (BlockType::Type(a), b) | (b, BlockType::Type(a)) => {
                self.results(b).is_some_and(|results| results == [*a])
            }
            _ => self
                .results(a)
                .zip(self.results(b))
                .is_some_and(|(a, b)| a == b),
        }
    }
}

impl ModuleArity for Stack<'_> {
    fn sub_type_at(&self, i: u32) -> Option<&SubType> {
        self.types.types.get(i as usize)
    }
    fn type_index_of_function(&self, i: u32) -> Option<u32> {
        self.types.functions.get(i as usize).copied()
    }
    fn tag_type_arity(&self, _: u32) -> Option<(u32, u32)> {
        None
    }
    fn func_type_of_cont_type(&self, _: &ContType) -> Option<&FuncType> {
        None
    }
    fn sub_type_of_ref_type(&self, _: &RefType) -> Option<&SubType> {
        None
    }
    fn control_stack_height(&self) -> u32 {
        self.frames.len() as u32
    }
    fn label_block(&self, depth: u32) -> Option<(BlockType, FrameKind)> {
        self.frames
            .get(self.frames.len().checked_sub(depth as usize + 1)?)
            .map(|f| (f.ty, f.kind))
    }
}

/// Operator positions to replace with nothing (false) or drop (true).
pub(super) fn find(
    body: &wasmparser::FunctionBody<'_>,
    types: &Types,
    defined: usize,
) -> Vec<(usize, bool)> {
    analyze(body, types, defined).unwrap_or_default()
}

fn analyze(
    body: &wasmparser::FunctionBody<'_>,
    types: &Types,
    defined: usize,
) -> Option<Vec<(usize, bool)>> {
    let signature = *types.functions.get(types.imported + defined)?;
    let CompositeInnerType::Func(function) = &types.types[signature as usize].composite_type.inner
    else {
        return None;
    };
    let results = function.results().len() as u32;
    let ops = body
        .get_operators_reader()
        .ok()?
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let mut stack = Stack {
        types,
        frames: vec![Frame {
            ty: BlockType::FuncType(signature),
            kind: FrameKind::Block,
            base: 0,
            dead: false,
        }],
    };
    let mut height = 0_u32;
    let mut changes = Vec::new();
    for (at, op) in ops.iter().enumerate() {
        let depth = match op {
            O::Return => Some((stack.frames.len() - 1, false)),
            O::Br { relative_depth } => Some((*relative_depth as usize, false)),
            O::BrIf { relative_depth } => Some((*relative_depth as usize, true)),
            _ => None,
        };
        if let Some((depth, conditional)) = depth {
            let first = stack.frames.len().checked_sub(depth + 1)?;
            let value_height = height.checked_sub(u32::from(conditional));
            let target = &stack.frames[first];
            let fallthrough = stack.frames[first..].iter().all(|frame| {
                let count = stack.block_type_arity(frame.ty).map(|(_, n)| n);
                frame.base == target.base
                    && stack.same_results(&frame.ty, &target.ty)
                    && count.is_some_and(|n| value_height == Some(frame.base + n))
            });
            if !stack.frames.last()?.dead
                && stack.frames[first].kind != FrameKind::Loop
                && fallthrough
                && ops
                    .get(at + 1..at + depth + 2)
                    .is_some_and(|suffix| suffix.iter().all(|op| matches!(op, O::End)))
            {
                changes.push((at, conditional));
            }
        }
        match op {
            O::Block { blockty } | O::Loop { blockty } | O::If { blockty } => {
                let (params, _) = stack.block_type_arity(*blockty)?;
                let parent = stack.frames.last()?;
                let condition = u32::from(matches!(op, O::If { .. }));
                if !parent.dead && height < parent.base + params + condition {
                    return None;
                }
                height = height.saturating_sub(condition).max(parent.base);
                let base = height.saturating_sub(params).max(parent.base);
                stack.frames.push(Frame {
                    ty: *blockty,
                    kind: if matches!(op, O::Loop { .. }) {
                        FrameKind::Loop
                    } else {
                        FrameKind::Block
                    },
                    base,
                    dead: parent.dead,
                });
                height = base + params;
            }
            O::Else => {
                let frame = stack.frames.last()?;
                height = frame.base + stack.block_type_arity(frame.ty)?.0;
                let parent_dead = stack.frames.get(stack.frames.len().checked_sub(2)?)?.dead;
                stack.frames.last_mut()?.dead = parent_dead;
            }
            O::End => {
                let frame = stack.frames.last()?;
                height = frame.base
                    + if stack.frames.len() == 1 {
                        results
                    } else {
                        stack.block_type_arity(frame.ty)?.1
                    };
                stack.frames.pop();
            }
            O::Unreachable
            | O::Return
            | O::Br { .. }
            | O::BrTable { .. }
            | O::ReturnCall { .. }
            | O::ReturnCallIndirect { .. }
            | O::ReturnCallRef { .. } => {
                let frame = stack.frames.last_mut()?;
                height = frame.base;
                frame.dead = true;
            }
            O::Try { .. }
            | O::TryTable { .. }
            | O::Delegate { .. }
            | O::Catch { .. }
            | O::CatchAll
            | O::Rethrow { .. }
            | O::Resume { .. }
            | O::ResumeThrow { .. }
            | O::ResumeThrowRef { .. }
            | O::Switch { .. }
            | O::Suspend { .. } => return None,
            _ => {
                let (inputs, outputs) = op.operator_arity(&stack)?;
                let frame = stack.frames.last()?;
                if !frame.dead && height < frame.base + inputs {
                    return None;
                }
                height = height.saturating_sub(inputs).max(frame.base) + outputs;
            }
        }
    }
    Some(changes)
}
