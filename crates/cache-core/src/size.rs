use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// Error type for size parsing failures.
#[derive(Error, Debug, PartialEq, Eq, Clone)]
pub enum SizeParseError {
    #[error("Invalid size string: empty or whitespace only")]
    Empty,
    #[error("Invalid numeric value in size: '{0}'")]
    InvalidNumber(String),
    #[error("Unknown or unsupported size unit: '{0}' (supported: B, KB, KiB, MB, MiB, GB, GiB, TB, TiB)")]
    UnknownUnit(String),
    #[error("Size value caused arithmetic overflow")]
    Overflow,
}

/// Represents a byte size with support for human-readable parsing (e.g. "500 MB", "2 GB", "10 GB")
/// and formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ByteSize(pub u64);

impl ByteSize {
    pub const B: u64 = 1;
    pub const KB: u64 = 1024;
    pub const MB: u64 = 1024 * 1024;
    pub const GB: u64 = 1024 * 1024 * 1024;
    pub const TB: u64 = 1024 * 1024 * 1024 * 1024;

    /// Creates a new `ByteSize` from raw bytes.
    pub const fn bytes(bytes: u64) -> Self {
        Self(bytes)
    }

    /// Creates a new `ByteSize` from kilobytes (1024 bytes).
    pub const fn kb(kb: u64) -> Self {
        Self(kb * Self::KB)
    }

    /// Creates a new `ByteSize` from megabytes (1024^2 bytes).
    pub const fn mb(mb: u64) -> Self {
        Self(mb * Self::MB)
    }

    /// Creates a new `ByteSize` from gigabytes (1024^3 bytes).
    pub const fn gb(gb: u64) -> Self {
        Self(gb * Self::GB)
    }

    /// Creates a new `ByteSize` from terabytes (1024^4 bytes).
    pub const fn tb(tb: u64) -> Self {
        Self(tb * Self::TB)
    }

    /// Returns the raw byte count as `u64`.
    pub const fn as_bytes(&self) -> u64 {
        self.0
    }

    /// Returns the size in kilobytes as a float (`f64`).
    pub fn as_kb_f64(&self) -> f64 {
        self.0 as f64 / Self::KB as f64
    }

    /// Returns the size in megabytes as a float (`f64`).
    pub fn as_mb_f64(&self) -> f64 {
        self.0 as f64 / Self::MB as f64
    }

    /// Returns the size in gigabytes as a float (`f64`).
    pub fn as_gb_f64(&self) -> f64 {
        self.0 as f64 / Self::GB as f64
    }

    /// Parses a human-readable size string into a `ByteSize`.
    ///
    /// Supported formats include:
    /// - `"500 MB"`, `"500MB"`, `"500 mb"`, `"500MiB"`
    /// - `"2 GB"`, `"2GB"`, `"2.5 GB"`
    /// - `"10 GB"`
    /// - `"1024 KB"`, `"1024"` (assumed bytes if no unit)
    pub fn parse(s: &str) -> Result<Self, SizeParseError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(SizeParseError::Empty);
        }

        // Find boundary between number and unit
        let num_end = trimmed
            .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
            .unwrap_or(trimmed.len());

        let (num_part, unit_part) = trimmed.split_at(num_end);
        let num_str = num_part.trim();
        let unit_str = unit_part.trim();

        if num_str.is_empty() {
            return Err(SizeParseError::InvalidNumber(s.to_string()));
        }

        // Parse number (support floating point e.g. "1.5 GB")
        let multiplier: f64 = if unit_str.is_empty() {
            1.0
        } else {
            let unit_lower = unit_str.to_lowercase();
            match unit_lower.as_str() {
                "b" | "bytes" | "byte" => 1.0,
                "k" | "kb" | "kib" | "kbytes" | "kilobytes" => Self::KB as f64,
                "m" | "mb" | "mib" | "mbytes" | "megabytes" => Self::MB as f64,
                "g" | "gb" | "gib" | "gbytes" | "gigabytes" => Self::GB as f64,
                "t" | "tb" | "tib" | "tbytes" | "terabytes" => Self::TB as f64,
                _ => return Err(SizeParseError::UnknownUnit(unit_str.to_string())),
            }
        };

        if let Ok(int_val) = num_str.parse::<u64>() {
            if multiplier == 1.0 {
                return Ok(Self(int_val));
            }
            let bytes_f = (int_val as f64) * multiplier;
            if bytes_f > u64::MAX as f64 || bytes_f < 0.0 {
                return Err(SizeParseError::Overflow);
            }
            Ok(Self(bytes_f as u64))
        } else if let Ok(float_val) = num_str.parse::<f64>() {
            if float_val < 0.0 {
                return Err(SizeParseError::InvalidNumber(num_str.to_string()));
            }
            let bytes_f = float_val * multiplier;
            if bytes_f > u64::MAX as f64 || bytes_f.is_nan() || bytes_f.is_infinite() {
                return Err(SizeParseError::Overflow);
            }
            Ok(Self(bytes_f.round() as u64))
        } else {
            Err(SizeParseError::InvalidNumber(num_str.to_string()))
        }
    }

    /// Formats the byte size into a human-readable string.
    pub fn to_human_readable(&self) -> String {
        let b = self.0;
        if b < Self::KB {
            format!("{} B", b)
        } else if b < Self::MB {
            format!("{:.2} KB", self.as_kb_f64())
        } else if b < Self::GB {
            format!("{:.2} MB", self.as_mb_f64())
        } else if b < Self::TB {
            format!("{:.2} GB", self.as_gb_f64())
        } else {
            format!("{:.2} TB", b as f64 / Self::TB as f64)
        }
    }
}

impl FromStr for ByteSize {
    type Err = SizeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Display for ByteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_human_readable())
    }
}

impl From<u64> for ByteSize {
    fn from(val: u64) -> Self {
        Self(val)
    }
}

impl From<ByteSize> for u64 {
    fn from(val: ByteSize) -> Self {
        val.0
    }
}

impl Serialize for ByteSize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(self.0)
    }
}

impl<'de> Deserialize<'de> for ByteSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum ByteSizeHelper {
            Num(u64),
            Str(String),
        }

        match ByteSizeHelper::deserialize(deserializer)? {
            ByteSizeHelper::Num(n) => Ok(ByteSize(n)),
            ByteSizeHelper::Str(s) => ByteSize::parse(&s).map_err(serde::de::Error::custom),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytesize_constants() {
        assert_eq!(ByteSize::B, 1);
        assert_eq!(ByteSize::KB, 1024);
        assert_eq!(ByteSize::MB, 1024 * 1024);
        assert_eq!(ByteSize::GB, 1024 * 1024 * 1024);
        assert_eq!(ByteSize::TB, 1024 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_bytesize_constructors() {
        assert_eq!(ByteSize::bytes(500).as_bytes(), 500);
        assert_eq!(ByteSize::kb(10).as_bytes(), 10 * 1024);
        assert_eq!(ByteSize::mb(500).as_bytes(), 500 * 1024 * 1024);
        assert_eq!(ByteSize::gb(2).as_bytes(), 2 * 1024 * 1024 * 1024);
        assert_eq!(ByteSize::gb(10).as_bytes(), 10 * 1024 * 1024 * 1024);
        assert_eq!(ByteSize::tb(1).as_bytes(), 1024 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_bytesize_parse_examples() {
        // Examples from Milestone 7.1 specification
        let s500mb = ByteSize::parse("500 MB").expect("valid 500 MB");
        assert_eq!(s500mb.as_bytes(), 500 * 1024 * 1024);

        let s2gb = ByteSize::parse("2 GB").expect("valid 2 GB");
        assert_eq!(s2gb.as_bytes(), 2 * 1024 * 1024 * 1024);

        let s10gb = ByteSize::parse("10 GB").expect("valid 10 GB");
        assert_eq!(s10gb.as_bytes(), 10 * 1024 * 1024 * 1024);

        // Case insensitivity & unit variations
        assert_eq!(
            ByteSize::parse("500MB").unwrap().as_bytes(),
            500 * 1024 * 1024
        );
        assert_eq!(
            ByteSize::parse("500 mb").unwrap().as_bytes(),
            500 * 1024 * 1024
        );
        assert_eq!(
            ByteSize::parse("500 MiB").unwrap().as_bytes(),
            500 * 1024 * 1024
        );
        assert_eq!(
            ByteSize::parse("2gb").unwrap().as_bytes(),
            2 * 1024 * 1024 * 1024
        );
        assert_eq!(
            ByteSize::parse("2 GiB").unwrap().as_bytes(),
            2 * 1024 * 1024 * 1024
        );
        assert_eq!(ByteSize::parse("1024 KB").unwrap().as_bytes(), 1024 * 1024);
        assert_eq!(ByteSize::parse("4096 B").unwrap().as_bytes(), 4096);
        assert_eq!(ByteSize::parse("4096").unwrap().as_bytes(), 4096);
    }

    #[test]
    fn test_bytesize_parse_fractions() {
        let size = ByteSize::parse("1.5 GB").expect("valid 1.5 GB");
        assert_eq!(size.as_bytes(), (1.5 * 1024.0 * 1024.0 * 1024.0) as u64);

        let size_mb = ByteSize::parse("0.5 MB").expect("valid 0.5 MB");
        assert_eq!(size_mb.as_bytes(), 512 * 1024);
    }

    #[test]
    fn test_bytesize_parse_errors() {
        assert_eq!(ByteSize::parse("").unwrap_err(), SizeParseError::Empty);
        assert_eq!(ByteSize::parse("   ").unwrap_err(), SizeParseError::Empty);
        assert!(matches!(
            ByteSize::parse("invalid").unwrap_err(),
            SizeParseError::InvalidNumber(_)
        ));
        assert!(matches!(
            ByteSize::parse("500 PB").unwrap_err(),
            SizeParseError::UnknownUnit(_)
        ));
        assert!(matches!(
            ByteSize::parse("-50 MB").unwrap_err(),
            SizeParseError::InvalidNumber(_)
        ));
    }

    #[test]
    fn test_bytesize_formatting_and_serde() {
        let size = ByteSize::gb(2);
        assert_eq!(size.to_human_readable(), "2.00 GB");
        assert_eq!(format!("{}", size), "2.00 GB");

        // Serde roundtrip as number
        let json_num = serde_json::to_string(&size).unwrap();
        assert_eq!(json_num, "2147483648");
        let de_num: ByteSize = serde_json::from_str(&json_num).unwrap();
        assert_eq!(de_num, size);

        // Serde deserialization from string
        let de_str: ByteSize = serde_json::from_str("\"2 GB\"").unwrap();
        assert_eq!(de_str, size);
    }
}
