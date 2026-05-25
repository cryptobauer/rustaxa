/// Timestamp or duration represented as whole microseconds.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Microseconds(
    /// Number of microseconds.
    pub u64,
);
