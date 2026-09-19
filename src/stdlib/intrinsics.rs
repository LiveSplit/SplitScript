//! Closed identities for compiler-provided standard-library operations.
//!
//! Unlike ordinary library declarations, these identities are part of the
//! compiler trust boundary. Privileged SplitScript source may bind a public
//! declaration to one of them, but cannot create a new intrinsic, redefine its
//! effects, or select its lowering and host dependencies.

macro_rules! trusted_intrinsics {
    ($consumer:ident) => {
        $consumer! {
            Print,
            TimerSetVariable,
            RuntimeSetTickRate,
            SettingsEnabled,
            SettingsContains,
            InstantNow,
            NextTick,
            FutureRace,
            FutureTimeout,
            FileReadAllBytes,
            FileReadAllText,
            BoolNot,
            IntegerBitNot,
            NumericSwapBytes,
            IntegerToStringRadix,
            NumericAdd,
            NumericSubtract,
            NumericMultiply,
            NumericDivide,
            IntegerRemainder,
            IntegerBitOr,
            IntegerBitXor,
            IntegerBitAnd,
            IntegerShiftLeft,
            IntegerShiftRight,
            EquatableEquals,
            EquatableNotEquals,
            SignedNegate,
            NumericMin,
            NumericMax,
            FloatSqrt,
            FloatTruncate,
            FloatFloor,
            FloatCeil,
            FloatRound,
            F32FromBits,
            F32ToBits,
            F64FromBits,
            F64ToBits,
            ArrayLength,
            ArraySet,
            ArrayPush,
            ArrayRemoveAt,
            ArrayClear,
            ArrayIterator,
            ArrayIteratorNext,
            SetIterator,
            SetIteratorNext,
            ExclusiveRangeIterator,
            ExclusiveRangeIteratorNext,
            InclusiveRangeIterator,
            InclusiveRangeIteratorNext,
            SetNew,
            SetLength,
            SetContains,
            SetInsert,
            SetRemove,
            SetClear,
            AddressAdd,
            ProcessName,
            ProcessPath,
            ProcessMainModule,
            ProcessClosed,
            ProcessModule,
            ProcessLoadedModule,
            ProcessFindMemoryRange,
            ProcessMemoryRanges,
            ProcessRead,
            ProcessFollow,
            ProcessScan,
            ProcessScanOnce,
            ModuleScanRelative32Target,
            ProcessScanMemory,
            ProcessScanMemoryAny,
            ProcessReadRelative32,
            MemoryReaderReadUtf8,
            MemoryReaderReadUtf16Le,
            TimerState,
            TimerCurrentSplitIndex,
            TimerSegmentWasSplit,
            TimerSkipSplit,
            TimerUndoSplit,
            TimerPauseGameTime,
            TimerResumeGameTime,
            RuntimeOperatingSystem,
            RuntimeArchitecture,
            StringLength,
            StringContains,
            StringIndexOf,
            StringLastIndexOf,
            StringStartsWith,
            StringEndsWith,
            StringEqualsIgnoreAsciiCase,
            StringToAsciiLowerCase,
            StringToAsciiUpperCase,
            StringTrimAsciiWhitespace,
            StringIsBlank,
            StringPadStart,
            StringPadEnd,
            StringReplaceAll,
            StringSplit,
            StringParse,
            StringByteAt,
            StringCharAt,
            StringSlice,
            StringConcat,
            StringJoin,
            ModuleScan,
            ModuleScanAny,
            ModulePath,
            ModuleMd5,
            GBAEmulatorRead,
            Ps2EmulatorRead,
            Ps1EmulatorRead,
            SmsEmulatorRead,
            GenesisEmulatorRead,
            GCNEmulatorRead,
            WiiEmulatorRead,
        }
    };
}

macro_rules! define_intrinsic_ids {
    ($($id:ident),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum IntrinsicId {
            $($id),*
        }

        impl IntrinsicId {
            pub const ALL: &'static [Self] = &[$(Self::$id),*];

            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$id => stringify!($id)),*
                }
            }

            pub fn named(name: &str) -> Option<Self> {
                match name {
                    $(stringify!($id) => Some(Self::$id),)*
                    _ => None,
                }
            }
        }
    };
}

trusted_intrinsics!(define_intrinsic_ids);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_intrinsic_names_round_trip() {
        for id in IntrinsicId::ALL {
            assert_eq!(IntrinsicId::named(id.name()), Some(*id));
        }
        assert_eq!(IntrinsicId::named("UserDefinedHostEscape"), None);
    }
}
