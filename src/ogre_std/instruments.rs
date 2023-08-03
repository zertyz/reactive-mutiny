//! Resting place for [Instruments] used across this module
//! TODO 2023-08-02: eventually, merge this and `./src/instruments.rs` together, into OgreStd, probably... making the instrumentation options general


/// Honors the *Zero-Cost Instrumentation Pattern* for the [ogre_std] containers:\
/// Designed to be used as a const generic parameter for Structs,
/// causes the conditional instrumentation code in client structs to
/// be selectively compiled -- see the implemented *const methods* for
/// documentation of each available instrument.
///
/// Note: Using this enum directly in generics --
/// as in `struct S<const INSTUMENTS: Instruments = { Instruments::NoInstruments }> {...}`
/// -- is not possible yet (as of Rust 1.64): *"adt const params is experimental [E0658]"*,
/// so we'll use it as `struct S<const INSTRUMENTS: usize = 0>` instead, and use [Instruments::from(INSTRUMENTS)]
/// in the instrumentable implementations & `let s = S::<Instruments::<YourOption>::into()> {};` in clients.
#[derive(Debug,Clone,Copy,PartialEq)]
#[repr(usize)]
pub enum Instruments {

    /// No conditional instrumentation code will be included -- bringing in the fastest
    /// possible execution speeds at the expense of lowest operational control
    Uninstrumented             = 0,

    /// Counters are updated, but no periodical output will be logged in the *INFO* level
    MetricsWithoutDiagnostics  = Self::METRICS,
    MetricsWithDiagnostics     = Self::METRICS | Self::METRICS_DIAGNOSTICS,
    OperationsTracing          = Self::TRACING,
    MetricsAndTracing          = Self::METRICS | Self::METRICS_DIAGNOSTICS | Self::TRACING,
}

impl Instruments {

    const METRICS:             usize = 1;
    const METRICS_DIAGNOSTICS: usize = 2;
    const TRACING:             usize = 4;

    /// To be used in if conditions, returns the enum variant
    /// that corresponds to the given const generic numeric value as described in [Self]
    pub const fn from(instruments: usize) -> Self {
        match instruments {
            0                                                         => Self::Uninstrumented,
            x if x == Self::MetricsWithoutDiagnostics as usize => Self::MetricsWithoutDiagnostics,
            x if x == Self::MetricsWithDiagnostics as usize    => Self::MetricsWithDiagnostics,
            x if x == Self::OperationsTracing as usize         => Self::OperationsTracing,
            x if x == Self::MetricsAndTracing as usize         => Self::MetricsAndTracing,
            _                                                         => {
                // panic!("Don't know how to create an `Instruments` enum variant from value {instruments}");
                Self::Uninstrumented
            },
        }
    }

    /// designed to be used by clients of the implementor structs, returns the number to be used as a
    /// const generic numeric value (when instantiating the implementor struct) that corresponds
    /// to the given enum variant
    pub const fn into(self) -> usize {
        match self {
            Self::Uninstrumented             => 0,
            Self::MetricsWithoutDiagnostics => Self::METRICS,
            Self::MetricsWithDiagnostics    => Self::METRICS | Self::METRICS_DIAGNOSTICS,
            Self::OperationsTracing         => Self::TRACING,
            Self::MetricsAndTracing         => Self::METRICS | Self::METRICS_DIAGNOSTICS | Self::TRACING,
        }
    }

    /// returns true if metrics should be computed
    pub const fn metrics(self) -> bool {
        matches!(self, Instruments::MetricsWithoutDiagnostics | Instruments::MetricsWithDiagnostics | Instruments::MetricsAndTracing)
    }

    /// returns true if operations should be traced
    pub const fn tracing(self) -> bool {
        matches!(self, Instruments::OperationsTracing | Instruments::MetricsAndTracing)
    }

    /// returns true if occasional diagnostic messages should be output with
    /// the current metrics counters
    pub const fn metrics_diagnostics(self) -> bool {
        matches!(self, Instruments::MetricsWithDiagnostics | Instruments::MetricsAndTracing)
    }
}

#[cfg(any(test,doc))]
mod tests {
    //! Unit tests for [container_instruments](super) module

    use super::*;


    #[cfg_attr(not(doc),test)]
    fn exhaustive_from_and_into_conversions() {
        let all_variants = [
            Instruments::Uninstrumented,
            Instruments::MetricsWithoutDiagnostics,
            Instruments::MetricsWithDiagnostics,
            Instruments::OperationsTracing,
            Instruments::MetricsAndTracing,
        ];
        for variant in all_variants {
            let number = variant.into();
            let reconverted = Instruments::from(number);
            println!("variant: {:?} -- usize: {}; reconverted to variant: {:?} -- .metrics()={}; .metrics_diagnostics()={}; .tracing()={}",
                     variant, number, reconverted,
                     variant.metrics(), variant.metrics_diagnostics(), variant.tracing());
            assert_eq!(reconverted, variant, ".into() / .from() failed -- intermediary number is {}", number);
        }
    }

    #[cfg_attr(not(doc),test)]
    fn invalid_from() {
        // as soon as Rust allows it, the code bellow should not even compile
        assert_eq!(Instruments::from(127), Instruments::Uninstrumented);
    }
}