/// To be used as a const generic parameter for containers,
/// causes code to be included to enable the instrumentations specified bellow
/// (see the implemented const methods for documentation on each instrument).
///
/// Note: Using this in generics is not possible yet (as of Rust 1.64) -- "adt const params is experimental [E0658]"
/// as in `struct S<const INSTUMENTS: ContainerInstruments = { ContainerInstruments::NoInstruments }> {...}`\
/// so we'll use it as `struct S<const INSTRUMENTS: usize = 0>` instead, and use [ContainerInstruments::from(INSTRUMENTS)]
/// in the container implementations & `let s = S::<ContainerInstruments::<YourOption>::into()> {};` in clients.
#[derive(Debug,Clone,Copy,PartialEq)]
pub enum ContainerInstruments {
    NoInstruments,              // = 0,
    MetricsWithoutDiagnostics,  // = metrics(),
    MetricsWithDiagnostics,     // = metrics() | metricsDiagnostics(),
    OperationsTracing,          // = tracing(),
    MetricsAndTracing,          // = metrics() | metricsDiagnostics() | tracing(),
}
impl ContainerInstruments {

    const METRICS:             usize = 1;
    const METRICS_DIAGNOSTICS: usize = 2;
    const TRACING:             usize = 4;

    /// designed to be used by container implementations, returns the enum variant
    /// that corresponds to the given const generic numeric value
    pub const fn from(instruments: usize) -> Self {
        match instruments {
            0                                                               => Self::NoInstruments,
            1 /*Self::METRICS*/                                             => Self::MetricsWithoutDiagnostics,
            3 /*Self::METRICS | Self::METRICS_DIAGNOSTICS*/                 => Self::MetricsWithDiagnostics,
            4 /*Self::TRACING*/                                             => Self::OperationsTracing,
            7 /*Self::METRICS | Self::METRICS_DIAGNOSTICS | Self::TRACING*/ => Self::MetricsAndTracing,
            _                                                         => {
                // panic!("Don't know how to create a `ContainerInstruments` enum variant from value {instruments}. Known constants are: METRICS={}; METRICS_DIAGNOSTICS={}; TRACING={}",
                //        Self::METRICS, Self::METRICS_DIAGNOSTICS, Self::TRACING);
                Self::NoInstruments
            },
        }
    }

    /// designed to be used by container clients, returns the number to be used as a
    /// const generic numeric value (when instantiating containers) that corresponds
    /// to the given enum variant
    pub const fn into(self) -> usize {
        match self {
            Self::NoInstruments             => 0,
            Self::MetricsWithoutDiagnostics => Self::METRICS,
            Self::MetricsWithDiagnostics    => Self::METRICS | Self::METRICS_DIAGNOSTICS,
            Self::OperationsTracing         => Self::TRACING,
            Self::MetricsAndTracing         => Self::METRICS | Self::METRICS_DIAGNOSTICS | Self::TRACING,
        }
    }

    /// returns true if metrics should be computed
    pub const fn metrics(self) -> bool {
        match self {
            ContainerInstruments::MetricsWithoutDiagnostics => true,
            ContainerInstruments::MetricsWithDiagnostics    => true,
            ContainerInstruments::MetricsAndTracing         => true,
            _                                               => false,
        }
    }

    /// returns true if operations should be traced
    pub const fn tracing(self) -> bool {
        match self {
            ContainerInstruments::OperationsTracing => true,
            ContainerInstruments::MetricsAndTracing => true,
            _                                       => false,
        }
    }

    /// returns true if occasional diagnostic messages should be output with
    /// the current metrics counters
    pub const fn metricsDiagnostics(self) -> bool {
        match self {
            ContainerInstruments::MetricsWithDiagnostics => true,
            ContainerInstruments::MetricsAndTracing      => true,
            _                                            => false,
        }
    }
}

#[cfg(any(test, feature="dox"))]
mod tests {
    //! Unit tests for [container_instruments](super) module

    use super::*;


    #[test]
    fn exhaustive_from_and_into_conversions() {
        let all_variants = [
            ContainerInstruments::NoInstruments,
            ContainerInstruments::MetricsWithoutDiagnostics,
            ContainerInstruments::MetricsWithDiagnostics,
            ContainerInstruments::OperationsTracing,
            ContainerInstruments::MetricsAndTracing,
        ];
        for variant in all_variants {
            let number = variant.into();
            let reconverted = ContainerInstruments::from(number);
            println!("variant: {:?} -- usize: {}; reconverted to variant: {:?} -- .metrics()={}; .metricsDiagnostics()={}; .tracing()={}",
                     variant, number, reconverted,
                     variant.metrics(), variant.metricsDiagnostics(), variant.tracing());
            assert_eq!(reconverted, variant, ".into() / .from() failed -- intermediary number is {}", number);
        }
    }

    #[test]
    fn invalid_from() {
        // as soon as Rust allows it, the code bellow should not even compile
        assert_eq!(ContainerInstruments::from(127), ContainerInstruments::NoInstruments);
    }
}