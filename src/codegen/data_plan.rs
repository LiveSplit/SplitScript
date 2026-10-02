use std::{cell::RefCell, collections::HashMap};

use wasm_encoder::{ConstExpr, DataSection};

use crate::{
    ast::{Program, SettingFileFilter, SettingKind, StateSource},
    signature::parse_signature,
    wasm_ir,
};

use super::memory_plan::LinearMemoryLayout;
use super::reachability::Reachability;
use super::{dependencies::BackendDependencies, runtime_helpers::float_format};

#[cfg(test)]
mod tests;

pub(super) struct StaticData {
    pub strings: StringPool,
    pub signatures: SignaturePool,
    pub float_format: Option<FloatFormatData>,
    layout: LinearMemoryLayout,
}

pub(super) struct FloatFormatData {
    pub pow10_significands: i32,
    bytes: Vec<u8>,
}

pub(super) struct StringPool {
    base: u32,
    bytes: Vec<u8>,
    entries: HashMap<String, (u32, u32)>,
}

/// Passive initializer bytes for GC strings, collected only during emission.
/// These bytes never occupy linear memory or require a runtime reader helper.
#[derive(Default)]
pub(super) struct GcStringLiterals {
    segment: Option<u32>,
    pool: RefCell<GcStringPool>,
}

#[derive(Default)]
struct GcStringPool {
    bytes: Vec<u8>,
    offsets: HashMap<String, u32>,
}

impl GcStringLiterals {
    pub fn enable(&mut self, segment: u32) {
        self.segment = Some(segment);
    }

    pub fn intern(&self, value: &str) -> Option<(u32, u32)> {
        let segment = self.segment?;
        let mut pool = self.pool.borrow_mut();
        let existing = pool.offsets.get(value).copied();
        let length = u32::try_from(value.len()).expect("GC strings must fit wasm32");
        let old_length = u32::try_from(pool.bytes.len()).expect("GC string data must fit wasm32");
        let offset = existing.unwrap_or(old_length);
        let new_length = if existing.is_some() {
            old_length
        } else {
            old_length
                .checked_add(length)
                .expect("GC string data must fit wasm32")
        };
        // The GC opcode and array type index have the same size in both forms.
        // Each fixed-array byte needs an i32.const and its signed LEB operand.
        let fixed = uleb_len(length)
            + value
                .bytes()
                .map(|byte| 1 + sleb_len(i32::from(byte)))
                .sum::<usize>();
        let from_data = 2 + sleb_len(offset as i32) + sleb_len(length as i32) + uleb_len(segment);
        let data_growth = if existing.is_some() {
            0
        } else if pool.bytes.is_empty() {
            // Passive flag, byte count, DataCount section, and segment-count
            // growth. Reserve four bytes for growth of the data-section size
            // prefix (the full u32 LEB range), whose old size is not known here.
            value.len() + 1 + uleb_len(length) + 2 + uleb_len(segment + 1) + uleb_len(segment + 1)
                - uleb_len(segment)
                + 4
        } else {
            value.len() + uleb_len(new_length) - uleb_len(old_length) + 4
        };
        if from_data + data_growth >= fixed {
            return None;
        }
        if existing.is_none() {
            pool.bytes.extend_from_slice(value.as_bytes());
            pool.offsets.insert(value.to_owned(), offset);
        }
        Some((segment, offset))
    }

    pub fn append_to(&self, data: &mut DataSection) -> bool {
        let pool = self.pool.borrow();
        if pool.bytes.is_empty() {
            return false;
        }
        assert_eq!(self.segment, Some(data.len()));
        data.passive(pool.bytes.iter().copied());
        true
    }
}

fn uleb_len(value: u32) -> usize {
    (32 - value.leading_zeros()).max(1).div_ceil(7) as usize
}

fn sleb_len(value: i32) -> usize {
    let magnitude = if value < 0 { !value } else { value } as u32;
    (33 - magnitude.leading_zeros()).div_ceil(7) as usize
}

#[derive(Clone, Copy)]
pub(super) struct SignatureEntry {
    pub needle: u32,
    pub mask: u32,
    pub len: u32,
}

#[derive(Default)]
pub(super) struct SignaturePool {
    base: u32,
    bytes: Vec<u8>,
    entries: HashMap<String, SignatureEntry>,
}

impl StaticData {
    pub fn collect(
        program: &Program,
        process_names: &[&str],
        automatic_shape: Option<&crate::shape_selection::ShapeSelectionPlan>,
        wasm_ir: &wasm_ir::Program,
        reachability: &Reachability,
        dependencies: &BackendDependencies,
    ) -> Self {
        let state = program.state.as_ref().expect("checked programs have state");
        let mut strings = StringPool::new();
        for process in process_names {
            strings.intern(process);
        }
        if let Some(plan) = automatic_shape {
            let report = plan.failure_report(program);
            for message in report.messages() {
                strings.intern(message);
            }
        }
        for field in state.all_fields() {
            if let StateSource::Pointer(path) = &field.source
                && let crate::ast::PointerPathBase::Module { name, .. } = &path.base
            {
                strings.intern(name);
            }
        }
        for setting in &program.settings {
            strings.intern(setting.runtime_key());
            strings.intern(&setting.description);
            if let Some(tooltip) = &setting.tooltip {
                strings.intern(tooltip);
            }
            match &setting.kind {
                SettingKind::Text { default, .. } => {
                    strings.intern(default);
                }
                SettingKind::Choice { options, .. } => {
                    for option in options {
                        strings.intern(&option.variant);
                        strings.intern(&option.description);
                    }
                }
                SettingKind::File { filters, .. } => {
                    for filter in filters {
                        match filter {
                            SettingFileFilter::Name {
                                description,
                                pattern,
                            } => {
                                if let Some(description) = description {
                                    strings.intern(description);
                                }
                                strings.intern(pattern);
                            }
                            SettingFileFilter::Mime { value: mime, .. } => {
                                strings.intern(mime);
                            }
                        }
                    }
                }
                SettingKind::Bool { .. } | SettingKind::Title { .. } => {}
            }
        }
        for expression in wasm_ir.expressions() {
            if !reachability.contains_expression(expression.id) {
                continue;
            }
            // Ordinary literals are emitted as GC arrays. Only module queries
            // pass an expression's literal directly to the linear-memory ABI.
            // Interning every reachable literal also retained discarded error
            // messages from Unity's fallible metadata readers.
            if let wasm_ir::ExpressionKind::Call { target, arguments } = &expression.kind
                && super::resolved_intrinsic(target)
                    == Some(crate::stdlib::IntrinsicId::ProcessModule)
            {
                let wasm_ir::ExpressionKind::String(name) = &wasm_ir
                    .expression(arguments[0])
                    .expect("module name belongs to Wasm IR")
                    .kind
                else {
                    unreachable!("module queries require a literal name")
                };
                strings.intern(name);
            }
        }
        let mut signatures = SignaturePool::new();
        for expression in wasm_ir.expressions() {
            if !reachability.contains_expression(expression.id) {
                continue;
            }
            if let wasm_ir::ExpressionKind::Signature(signature) = &expression.kind {
                signatures.intern(signature);
            }
        }
        let float_format_bytes = if dependencies.uses_float_format() {
            float_format::pow10_significands_bytes()
        } else {
            Vec::new()
        };
        let static_data_len = strings
            .bytes
            .len()
            .checked_add(signatures.bytes.len())
            .and_then(|length| length.checked_add(float_format_bytes.len()))
            .expect("static data length must fit the host address space");
        let layout = LinearMemoryLayout::plan(
            static_data_len,
            dependencies.scratch_requirements(signatures.maximum_len()),
        );
        strings.base = layout.static_data_start();
        signatures.base = strings
            .base
            .checked_add(
                u32::try_from(strings.bytes.len())
                    .expect("string data must fit WebAssembly linear memory"),
            )
            .expect("string data must fit WebAssembly linear memory");
        let float_format = (!float_format_bytes.is_empty()).then(|| FloatFormatData {
            pow10_significands: i32::try_from(signatures.base as usize + signatures.bytes.len())
                .expect("float-format data address must fit wasm32"),
            bytes: float_format_bytes,
        });
        Self {
            strings,
            signatures,
            float_format,
            layout,
        }
    }

    pub fn layout(&self) -> LinearMemoryLayout {
        self.layout
    }

    pub fn segment_count(&self) -> u32 {
        u32::from(!self.strings.bytes.is_empty())
            + u32::from(!self.signatures.bytes.is_empty())
            + u32::from(self.float_format.is_some())
    }

    pub fn encode(&self) -> DataSection {
        debug_assert_eq!(
            self.layout.static_data_end(),
            u64::from(self.signatures.base)
                + self.signatures.bytes.len() as u64
                + self
                    .float_format
                    .as_ref()
                    .map_or(0, |float_format| float_format.bytes.len() as u64)
        );
        let mut section = DataSection::new();
        if !self.strings.bytes.is_empty() {
            section.active(
                0,
                &ConstExpr::i32_const(self.strings.base as i32),
                self.strings.bytes.iter().copied(),
            );
        }
        if !self.signatures.bytes.is_empty() {
            section.active(
                0,
                &ConstExpr::i32_const(self.signatures.base as i32),
                self.signatures.bytes.iter().copied(),
            );
        }
        if let Some(float_format) = &self.float_format {
            section.active(
                0,
                &ConstExpr::i32_const(float_format.pow10_significands),
                float_format.bytes.iter().copied(),
            );
        }
        section
    }
}

impl SignaturePool {
    fn new() -> Self {
        Self::default()
    }

    fn intern(&mut self, signature: &str) {
        if self.entries.contains_key(signature) {
            return;
        }
        let (needle, mask) = parse_signature(signature).expect("signatures were type checked");
        let needle_ptr = self.base + self.bytes.len() as u32;
        self.bytes.extend_from_slice(&needle);
        let mask_ptr = self.base + self.bytes.len() as u32;
        self.bytes.extend_from_slice(&mask);
        self.entries.insert(
            signature.to_owned(),
            SignatureEntry {
                needle: needle_ptr,
                mask: mask_ptr,
                len: needle.len() as u32,
            },
        );
    }

    pub fn get(&self, signature: &str) -> SignatureEntry {
        let entry = self.entries[signature];
        SignatureEntry {
            needle: self
                .base
                .checked_add(entry.needle)
                .expect("signature address must fit wasm32"),
            mask: self
                .base
                .checked_add(entry.mask)
                .expect("signature address must fit wasm32"),
            len: entry.len,
        }
    }

    fn maximum_len(&self) -> u32 {
        self.entries
            .values()
            .map(|entry| entry.len)
            .max()
            .unwrap_or(0)
    }
}

impl StringPool {
    fn new() -> Self {
        Self {
            base: 0,
            bytes: Vec::new(),
            entries: HashMap::new(),
        }
    }

    fn intern(&mut self, value: &str) -> (u32, u32) {
        if let Some(entry) = self.entries.get(value) {
            return *entry;
        }
        let offset = self
            .base
            .checked_add(
                u32::try_from(self.bytes.len())
                    .expect("string data must fit WebAssembly linear memory"),
            )
            .expect("string data must fit WebAssembly linear memory");
        let len = value.len() as u32;
        self.bytes.extend_from_slice(value.as_bytes());
        self.entries.insert(value.to_owned(), (offset, len));
        (offset, len)
    }

    pub fn get(&self, value: &str) -> (u32, u32) {
        let (offset, len) = self.entries[value];
        (
            self.base
                .checked_add(offset)
                .expect("string address must fit wasm32"),
            len,
        )
    }
}
