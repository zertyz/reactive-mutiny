//! Resting place for [Instruments] used across this module
//! TODO 2023-08-02: eventually, merge this and `./src/ogre_std/instruments.rs` together, into OgreStd, probably... making the instrumentation options general


/// Honors the *Zero-Cost Instrumentation Pattern* for the [stream_executors]:\
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
    NoInstruments               = 0,

    LogsWithoutMetrics          = Self::LOG,
    LogsWithMetrics             = Self::LOG | Self::PERFORMANCE_LOG | Self::COUNTERS | Self::SATURATION | Self::CHEAP_PROFILING,
    LogsWithExpensiveMetrics    = Self::LOG | Self::PERFORMANCE_LOG | Self::COUNTERS | Self::SATURATION | Self::EXPENSIVE_PROFILING,
    MetricsWithoutLogs          = Self::COUNTERS | Self::SATURATION | Self::CHEAP_PROFILING,
    ExpensiveMetricsWithoutLogs = Self::COUNTERS | Self::SATURATION | Self::EXPENSIVE_PROFILING,
    Custom(usize),
}

impl Instruments {

    /// metrics: counts every work done & outcome: `processed_events`, `untreated_failures`
    const COUNTERS:            usize = 1;
    /// metrics: keeps track of `avg_buffer_depth`, `max_depth`, serving cheap euristics to avoid needing to use [Instruments::EXPENSIVE_PROFILING]
    const SATURATION:          usize = 2;
    /// keeps track of averages for the execution duration of each received `Future`
    /// -- from the moment the executor starts them to their completion\
    /// (does nothing if the executor is not handling `Future`s)
    const CHEAP_PROFILING:     usize = 4;
    /// keeps track of averages for the total time each event takes to complete
    /// -- from the moment they were *produced* to the moment their *consumption* completes\
    /// (causes an increase in the payload due to the addition of the `u64` *start_time* field)
    const EXPENSIVE_PROFILING: usize = 8;
    /// metrics: computes counters of logarithmic decreasing precision of processing times for [Instruments::CHEAP_PROFILING] & [Instruments::EXPENSIVE_PROFILING], if enabled
    /// -- useful for computing processing/response time percentiles.\
    /// The map is in the form:\
    ///     `counters := {[time] = count, ...}`\
    /// where *time* increase with the power of 2 and counts all requests whose times are between the previous *time* and the current one.\
    /// provides: `futures_times` && `duration_times`
    const _PERCENTILES:         usize = 16;
    /// outputs `INFO` on stream's execution start & finish
    const LOG:                 usize = 32;
    /// if [Instruments::LOG] is enabled, periodically outputs information about all enabled metrics & profiling data collected
    const PERFORMANCE_LOG:     usize = 64;
    /// if [Instruments::LOG] is enabled, outputs every event in the *TRACE* level
    const TRACING:             usize = 128;

    /// To be used in if conditions, returns the enum variant
    /// that corresponds to the given const generic numeric value as described in [Self]
    pub const fn from(instruments: usize) -> Self {
        Self::Custom(instruments)
    }

    /// designed to be used by clients of the implementor structs, returns the number to be used as a
    /// const generic numeric value (when instantiating the implementor struct) that corresponds
    /// to the given enum variant
    pub const fn into(self) -> usize {
        match self {
            Self::NoInstruments               => 0,
            Self::LogsWithoutMetrics          => Self::LOG,
            Self::LogsWithMetrics             => Self::LOG | Self::PERFORMANCE_LOG | Self::COUNTERS | Self::SATURATION | Self::CHEAP_PROFILING,
            Self::LogsWithExpensiveMetrics    => Self::LOG | Self::PERFORMANCE_LOG | Self::COUNTERS | Self::SATURATION | Self::EXPENSIVE_PROFILING,
            Self::MetricsWithoutLogs          => Self::COUNTERS | Self::SATURATION | Self::CHEAP_PROFILING,
            Self::ExpensiveMetricsWithoutLogs => Self::COUNTERS | Self::SATURATION | Self::EXPENSIVE_PROFILING,
            Self::Custom(instruments)  => instruments,
        }
    }

    /// returns whether executors should log start/stop in the `INFO` level OR events `TRACING`
    pub const fn logging(self) -> bool {
        self.into() & (Self::LOG | Self::TRACING) > 0
    }

    /// returns whether executors should log events in the `TRACING` level
    pub const fn tracing(self) -> bool {
        self.into() & Self::TRACING > 0
    }

    /// returns whether executors should compute the `CHEAP_PROFILING` metrics
    pub const fn cheap_profiling(self) -> bool {
        self.into() & (Self::COUNTERS | Self::SATURATION | Self::CHEAP_PROFILING | Self::EXPENSIVE_PROFILING) > 0
    }

        /// returns whether executors should compute any of the available metrics
    pub const fn metrics(self) -> bool {
        self.into() & (Self::COUNTERS | Self::SATURATION | Self::CHEAP_PROFILING | Self::EXPENSIVE_PROFILING) > 0
    }



}

