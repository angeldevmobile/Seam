//! Named string formats, checked by hand.
//!
//! There is no regular expression here, and that is the design rather than an
//! omission. A general `@pattern(...)` would need a regex engine: a
//! backtracking one lets a hostile schema or a hostile payload burn unbounded
//! time, which breaks the promise that input is bounded, and a linear one
//! means the engine's first dependency plus its weight in every host — a crate
//! larger than the whole of `seam-core`, carried into a browser bundle that is
//! currently 113 KiB.
//!
//! A closed set of names costs neither. It also says something a pattern
//! cannot: `@format(uuid)` states what the value *is*, while a regex states
//! what it looks like, and only the first survives someone tightening the
//! pattern later.
//!
//! Each check below documents what it does **not** enforce. A format that
//! quietly rejects legitimate values is worse than no format at all, because
//! the failure lands on a user who is holding a perfectly good address.

/// A named format a string must satisfy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `8-4-4-4-12` lowercase or uppercase hex. Any version, any variant:
    /// version bits are a fact about who generated it, not about whether the
    /// value is a UUID.
    Uuid,
    /// A structural check only: one `@`, a non-empty local part, and a domain
    /// that looks like a hostname.
    ///
    /// Deliberately not RFC 5322, which admits comments, quoted strings and
    /// nested parentheses that no mail system in use accepts. Deliberately not
    /// a deliverability check either — that needs the network, and a validator
    /// that reached the network at a boundary would be a much worse idea than
    /// a permissive check.
    Email,
    /// A DNS hostname per RFC 1123: dot-separated labels of letters, digits
    /// and hyphens, each 1 to 63 characters, not starting or ending with a
    /// hyphen, 253 characters in total.
    Hostname,
    /// Four decimal octets. Leading zeros are rejected, because `010` is octal
    /// to some resolvers and decimal to others, and a value that means two
    /// different things on two hosts is exactly what this project exists to
    /// refuse.
    Ipv4,
    /// An IPv6 address, including the `::` short form and a trailing IPv4
    /// part. Zone identifiers (`%eth0`) are rejected: they name an interface
    /// on one machine and mean nothing on another.
    Ipv6,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Uuid => "uuid",
            Format::Email => "email",
            Format::Hostname => "hostname",
            Format::Ipv4 => "ipv4",
            Format::Ipv6 => "ipv6",
        }
    }

    pub fn parse(name: &str) -> Option<Format> {
        match name {
            "uuid" => Some(Format::Uuid),
            "email" => Some(Format::Email),
            "hostname" => Some(Format::Hostname),
            "ipv4" => Some(Format::Ipv4),
            "ipv6" => Some(Format::Ipv6),
            _ => None,
        }
    }

    /// Every name, in declaration order, for an error message that tells the
    /// author what they could have written instead.
    pub const ALL: [Format; 5] = [
        Format::Uuid,
        Format::Email,
        Format::Hostname,
        Format::Ipv4,
        Format::Ipv6,
    ];

    pub fn matches(self, value: &str) -> bool {
        match self {
            Format::Uuid => uuid(value),
            Format::Email => email(value),
            Format::Hostname => hostname(value),
            Format::Ipv4 => ipv4(value),
            Format::Ipv6 => ipv6(value),
        }
    }
}

fn uuid(v: &str) -> bool {
    let groups = [8, 4, 4, 4, 12];
    let mut parts = v.split('-');
    for len in groups {
        match parts.next() {
            Some(p) if p.len() == len && p.bytes().all(|b| b.is_ascii_hexdigit()) => {}
            _ => return false,
        }
    }
    parts.next().is_none()
}

fn email(v: &str) -> bool {
    // Split at the last `@`: a local part may contain one, a domain may not.
    let Some(at) = v.rfind('@') else {
        return false;
    };
    let (local, domain) = (&v[..at], &v[at + 1..]);

    if local.is_empty() || local.len() > 64 {
        return false;
    }
    // No control characters, no spaces, and nothing that would need quoting.
    // A quoted local part is legal and essentially unused; rejecting it is a
    // documented limit rather than an accident.
    if local
        .bytes()
        .any(|b| b <= b' ' || b == b'"' || b == b'\\' || b == b'@' || b == 0x7f)
    {
        return false;
    }
    if local.starts_with('.') || local.ends_with('.') || local.contains("..") {
        return false;
    }
    // A domain without a dot is syntactically fine and never routable from
    // outside its own network, which at an API boundary is a typo every time.
    hostname(domain) && domain.contains('.')
}

fn hostname(v: &str) -> bool {
    if v.is_empty() || v.len() > 253 {
        return false;
    }
    v.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

fn ipv4(v: &str) -> bool {
    let mut octets = 0;
    for part in v.split('.') {
        octets += 1;
        if octets > 4 || part.is_empty() || part.len() > 3 {
            return false;
        }
        if !part.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        // `010` is octal to some resolvers and decimal to others.
        if part.len() > 1 && part.starts_with('0') {
            return false;
        }
        if part.parse::<u16>().map_or(true, |n| n > 255) {
            return false;
        }
    }
    octets == 4
}

fn ipv6(v: &str) -> bool {
    // A zone identifier names an interface on one machine and nothing on
    // another, so it is not part of a portable address.
    if v.contains('%') {
        return false;
    }

    // At most one `::`, which stands for one or more groups of zeros.
    let halves: Vec<&str> = v.split("::").collect();
    let (head, tail, elided) = match halves.as_slice() {
        [whole] => (*whole, "", false),
        [before, after] => (*before, *after, true),
        _ => return false,
    };

    let groups = |s: &str| -> Option<(usize, bool)> {
        if s.is_empty() {
            return Some((0, false));
        }
        let parts: Vec<&str> = s.split(':').collect();
        let mut count = 0;
        let mut trailing_v4 = false;
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            // The final group may be a dotted IPv4 address, which occupies two.
            if last && part.contains('.') {
                if !ipv4(part) {
                    return None;
                }
                count += 2;
                trailing_v4 = true;
                continue;
            }
            if part.is_empty() || part.len() > 4 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            count += 1;
        }
        Some((count, trailing_v4))
    };

    let Some((left, left_v4)) = groups(head) else {
        return false;
    };
    let Some((right, right_v4)) = groups(tail) else {
        return false;
    };
    // An embedded IPv4 part only ever ends the address.
    if left_v4 && (elided || !tail.is_empty()) {
        return false;
    }
    let total = left + right;

    if elided {
        // `::` must stand for at least one group, or it would be a plain `:`.
        total < 8
    } else {
        total == 8 && !right_v4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids() {
        assert!(Format::Uuid.matches("6ba7b810-9dad-11d1-80b4-00c04fd430c8"));
        assert!(Format::Uuid.matches("6BA7B810-9DAD-11D1-80B4-00C04FD430C8"));
        // Any version and variant: those bits say who made it, not whether it
        // is one.
        assert!(Format::Uuid.matches("00000000-0000-0000-0000-000000000000"));

        assert!(!Format::Uuid.matches("6ba7b810-9dad-11d1-80b4-00c04fd430c"));
        assert!(!Format::Uuid.matches("6ba7b8109dad11d180b400c04fd430c8"));
        assert!(!Format::Uuid.matches("6ba7b810-9dad-11d1-80b4-00c04fd430c8-"));
        assert!(!Format::Uuid.matches("gba7b810-9dad-11d1-80b4-00c04fd430c8"));
        assert!(!Format::Uuid.matches(""));
    }

    #[test]
    fn emails() {
        assert!(Format::Email.matches("gabriel@example.com"));
        assert!(Format::Email.matches("first.last+tag@sub.example.co.uk"));
        assert!(Format::Email.matches("a@b.co"));

        assert!(!Format::Email.matches("no-at-sign.example.com"));
        assert!(!Format::Email.matches("@example.com"));
        assert!(!Format::Email.matches("user@"));
        assert!(!Format::Email.matches("user@localhost"));
        assert!(!Format::Email.matches("user name@example.com"));
        assert!(!Format::Email.matches(".user@example.com"));
        assert!(!Format::Email.matches("user..name@example.com"));
    }

    #[test]
    fn hostnames() {
        assert!(Format::Hostname.matches("example.com"));
        assert!(Format::Hostname.matches("localhost"));
        assert!(Format::Hostname.matches("a-b.example.com"));

        assert!(!Format::Hostname.matches(""));
        assert!(!Format::Hostname.matches("-example.com"));
        assert!(!Format::Hostname.matches("example-.com"));
        assert!(!Format::Hostname.matches("exa mple.com"));
        assert!(!Format::Hostname.matches("example..com"));
    }

    #[test]
    fn ipv4_addresses() {
        assert!(Format::Ipv4.matches("192.168.0.1"));
        assert!(Format::Ipv4.matches("0.0.0.0"));
        assert!(Format::Ipv4.matches("255.255.255.255"));

        assert!(!Format::Ipv4.matches("256.0.0.1"));
        assert!(!Format::Ipv4.matches("1.2.3"));
        assert!(!Format::Ipv4.matches("1.2.3.4.5"));
        // Octal to some resolvers, decimal to others.
        assert!(!Format::Ipv4.matches("010.0.0.1"));
        assert!(!Format::Ipv4.matches("1.2.3.-4"));
    }

    #[test]
    fn ipv6_addresses() {
        assert!(Format::Ipv6.matches("2001:0db8:85a3:0000:0000:8a2e:0370:7334"));
        assert!(Format::Ipv6.matches("2001:db8:85a3::8a2e:370:7334"));
        assert!(Format::Ipv6.matches("::1"));
        assert!(Format::Ipv6.matches("::"));
        assert!(Format::Ipv6.matches("::ffff:192.168.0.1"));

        assert!(!Format::Ipv6.matches("2001:db8::85a3::7334"));
        assert!(!Format::Ipv6.matches("2001:db8:85a3:0:0:8a2e:370"));
        assert!(!Format::Ipv6.matches("gggg::1"));
        // A zone identifier names an interface on one machine only.
        assert!(!Format::Ipv6.matches("fe80::1%eth0"));
        assert!(!Format::Ipv6.matches(""));
    }

    #[test]
    fn names_round_trip() {
        for f in Format::ALL {
            assert_eq!(Format::parse(f.name()), Some(f));
        }
        assert_eq!(Format::parse("regex"), None);
    }
}
