/// `Error` — see variants.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Error {
    // The value overflowed.
    /// `Overflow` variant.
    Overflow,

    // The input string was empty.
    /// `Empty` variant.
    Empty,

    // The input string contained an invalid char.
    // Note this may not be returned for conversions which stop at invalid chars.
    /// `InvalidChar` variant.
    InvalidChar,
}
