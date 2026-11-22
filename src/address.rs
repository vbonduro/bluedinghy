//! Addresses a message can come from or be sent to.
//!
//! BlueFerry identifies a party by a phone number, an email address (iMessage)
//! or an SMS short code. [`Address::parse`] classifies a raw string the same way
//! BlueFerry's `canonical_address` does.

use std::fmt;
use std::str::FromStr;

/// What kind of party an [`Address`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum AddressKind {
    /// A phone number, held in E.164 form (`+14165550123`).
    Phone,
    /// An email address, held lowercased.
    Email,
    /// A 3-6 digit SMS short code, e.g. `22000`.
    ShortCode,
}

/// Who a message is from, or where it is going.
///
/// Held in BlueFerry's canonical form, so two addresses are equal exactly when
/// BlueFerry would treat them as the same party: `+14165550123` equals
/// `+1 (416) 555-0123`, and email case doesn't matter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address {
    kind: AddressKind,
    canonical: String,
}

/// A string that couldn't be used as an [`Address`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a phone number (+country form), email address or short code: {0:?}")]
pub struct AddressError(String);

impl AddressError {
    /// The rejected input.
    pub fn input(&self) -> &str {
        &self.0
    }
}

impl Address {
    /// Classifies `raw` as an email, short code or phone number (checked in that
    /// order).
    ///
    /// Phone numbers must start with `+` and their country code, e.g.
    /// `+14165550123`. Spaces, dashes, dots and parentheses are ignored. This
    /// checks that the string is shaped like an E.164 number (7 to 15 digits, no
    /// leading zero), not that the number exists.
    pub fn parse(raw: &str) -> Result<Self, AddressError> {
        let raw = raw.trim();
        let (kind, canonical) = if is_email_shaped(raw) {
            (AddressKind::Email, raw.to_lowercase())
        } else if is_short_code(raw) {
            (AddressKind::ShortCode, raw.to_string())
        } else if let Some(phone) = normalize_phone(raw) {
            (AddressKind::Phone, phone)
        } else {
            return Err(AddressError(raw.to_string()));
        };
        Ok(Self { kind, canonical })
    }

    /// What kind of address this is.
    pub fn kind(&self) -> AddressKind {
        self.kind
    }

    /// The canonical string: E.164 for phones, lowercased for email. This is the
    /// form BlueFerry's `Send` expects as a recipient, and is suitable for storing.
    pub fn as_str(&self) -> &str {
        &self.canonical
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical)
    }
}

impl FromStr for Address {
    type Err = AddressError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// `+` and 7-15 digits, ignoring the separators BlueFerry tolerates. Returns the
/// number in E.164 form.
fn normalize_phone(s: &str) -> Option<String> {
    let rest = s.strip_prefix('+')?;
    if !rest
        .chars()
        .all(|c| c.is_ascii_digit() || " ()-.".contains(c))
    {
        return None;
    }
    let digits: String = rest.chars().filter(char::is_ascii_digit).collect();
    ((7..=15).contains(&digits.len()) && !digits.starts_with('0')).then(|| format!("+{digits}"))
}

fn is_short_code(s: &str) -> bool {
    (3..=6).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

/// Deliberately loose: one `@`, a non-empty local part, a dotted-or-plain domain
/// made of alphanumerics, `.` and `-`. Mirrors BlueFerry's `_EMAIL_SHAPED`.
fn is_email_shaped(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+/=?^_`{|}~.-".contains(c));
    let domain_ok = !domain.is_empty()
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && domain.starts_with(|c: char| c.is_ascii_alphanumeric())
        && domain.ends_with(|c: char| c.is_ascii_alphanumeric());
    local_ok && domain_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_e164_phone() {
        let a = Address::parse("+14165550123").unwrap();
        assert_eq!(a.kind(), AddressKind::Phone);
        assert_eq!(a.as_str(), "+14165550123");
    }

    #[test]
    fn ignores_phone_formatting() {
        let a = Address::parse(" +1 (416) 555-0123 ").unwrap();
        assert_eq!(a.as_str(), "+14165550123");
        assert_eq!(Address::parse("+1.416.555.0123").unwrap(), a);
    }

    #[test]
    fn same_number_written_two_ways_is_equal_and_hashes_alike() {
        use std::collections::HashSet;
        let plain = Address::parse("+14165550123").unwrap();
        let formatted = Address::parse("+1 (416) 555-0123").unwrap();
        assert_eq!(plain, formatted);
        assert_eq!(HashSet::from([plain]), HashSet::from([formatted]));
    }

    #[test]
    fn phone_without_country_code_fails() {
        assert!(Address::parse("(416) 555-0123").is_err());
        assert!(Address::parse("4165550123").is_err());
    }

    #[test]
    fn rejects_phone_numbers_with_a_bad_shape() {
        assert!(Address::parse("+1416").is_err()); // too short
        assert!(Address::parse("+1234567890123456").is_err()); // 16 digits
        assert!(Address::parse("+0165550123").is_err()); // leading zero
        assert!(Address::parse("+1416abc0123").is_err()); // letters
    }

    #[test]
    fn error_keeps_the_rejected_input() {
        assert_eq!(Address::parse(" Mom ").unwrap_err().input(), "Mom");
    }

    #[test]
    fn parses_email_lowercased() {
        let a = Address::parse(" Some.One@Example.COM ").unwrap();
        assert_eq!(a.kind(), AddressKind::Email);
        assert_eq!(a.as_str(), "some.one@example.com");
    }

    #[test]
    fn parses_short_code() {
        let a = Address::parse("22000").unwrap();
        assert_eq!(a.kind(), AddressKind::ShortCode);
        assert_eq!(a.as_str(), "22000");
    }

    #[test]
    fn rejects_names_and_empty() {
        assert!(Address::parse("Mom").is_err());
        assert!(Address::parse("").is_err());
        assert!(Address::parse("a@").is_err());
    }
}
