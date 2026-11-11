//! Variable scopes and `{{placeholder}}` resolution.
//!
//! Precedence (narrowest wins): local > data > environment > collection > globals.
//! `{{$name}}` are Postman dynamic variables.

use crate::model::VarMap;
use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Scopes {
    #[serde(default)]
    pub globals: VarMap,
    #[serde(default)]
    pub collection: VarMap,
    #[serde(default)]
    pub environment: VarMap,
    #[serde(default)]
    pub data: VarMap,
    #[serde(default)]
    pub local: VarMap,
}

impl Scopes {
    pub fn get(&self, name: &str) -> Option<&String> {
        self.local
            .get(name)
            .or_else(|| self.data.get(name))
            .or_else(|| self.environment.get(name))
            .or_else(|| self.collection.get(name))
            .or_else(|| self.globals.get(name))
    }

    /// Replace every `{{name}}`; unknown names are left untouched.
    pub fn resolve(&self, input: &str) -> String {
        self.resolve_depth(input, 0)
    }

    fn resolve_depth(&self, input: &str, depth: usize) -> String {
        if !input.contains("{{") {
            return input.to_string();
        }
        let mut out = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            match after.find("}}") {
                Some(end) => {
                    let name = after[..end].trim();
                    let value = if let Some(d) = name.strip_prefix('$') {
                        dynamic(d)
                    } else {
                        self.get(name).map(|v| if depth < 10 { self.resolve_depth(v, depth + 1) } else { v.clone() })
                    };
                    match value {
                        Some(v) => out.push_str(&v),
                        None => {
                            out.push_str("{{");
                            out.push_str(&after[..end]);
                            out.push_str("}}");
                        }
                    }
                    rest = &after[end + 2..];
                }
                None => {
                    out.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// Names referenced in `input` that cannot be resolved.
    pub fn unresolved(&self, input: &str) -> Vec<String> {
        let resolved = self.resolve(input);
        let mut out = vec![];
        let mut rest = resolved.as_str();
        while let Some(s) = rest.find("{{") {
            let after = &rest[s + 2..];
            match after.find("}}") {
                Some(e) => {
                    out.push(after[..e].trim().to_string());
                    rest = &after[e + 2..];
                }
                None => break,
            }
        }
        out
    }
}

const FIRST: &[&str] = &["Alice", "Bob", "Carol", "David", "Emma", "Frank", "Grace", "Henry", "Irene", "Jack", "Karen", "Liam", "Mia", "Noah", "Olivia", "Paul"];
const LAST: &[&str] = &["Smith", "Johnson", "Williams", "Brown", "Jones", "Garcia", "Miller", "Davis", "Wilson", "Moore", "Taylor", "Anderson"];
const WORDS: &[&str] = &["alpha", "bravo", "matrix", "pixel", "driver", "panel", "system", "array", "protocol", "bandwidth", "sensor", "monitor", "circuit", "feed", "port", "bus", "card", "program", "firewall", "interface"];
const COLORS: &[&str] = &["red", "green", "blue", "yellow", "purple", "orange", "black", "white", "teal", "magenta", "cyan", "silver"];
const TLDS: &[&str] = &["com", "net", "org", "io", "dev", "info"];
const COUNTRIES: &[&str] = &["Germany", "France", "Japan", "Brazil", "Canada", "Kenya", "India", "Norway", "Chile", "Australia"];
const CITIES: &[&str] = &["Berlin", "Paris", "Tokyo", "Lima", "Toronto", "Nairobi", "Mumbai", "Oslo", "Santiago", "Sydney"];
const JOBS: &[&str] = &["Engineer", "Designer", "Analyst", "Manager", "Architect", "Consultant", "Developer", "Director"];
const LOREM: &[&str] = &["lorem", "ipsum", "dolor", "sit", "amet", "consectetur", "adipiscing", "elit", "sed", "do", "eiusmod", "tempor", "incididunt", "ut", "labore"];

fn pick(list: &[&str]) -> String {
    list.choose(&mut rand::thread_rng()).copied().unwrap_or("").to_string()
}

fn chars(n: usize, set: &[u8]) -> String {
    let mut r = rand::thread_rng();
    (0..n).map(|_| set[r.gen_range(0..set.len())] as char).collect()
}

/// Postman dynamic variables (`{{$guid}}`, `{{$randomInt}}`, ...).
pub fn dynamic(name: &str) -> Option<String> {
    let mut r = rand::thread_rng();
    let now = chrono::Utc::now();
    Some(match name {
        "guid" | "randomUUID" => uuid::Uuid::new_v4().to_string(),
        "timestamp" => now.timestamp().to_string(),
        "isoTimestamp" => now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "randomInt" => r.gen_range(0..=1000).to_string(),
        "randomBoolean" => r.gen_bool(0.5).to_string(),
        "randomAlphaNumeric" => chars(1, b"abcdefghijklmnopqrstuvwxyz0123456789"),
        "randomPassword" => chars(15, b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"),
        "randomHexColor" => format!("#{}", chars(6, b"0123456789abcdef")),
        "randomColor" => pick(COLORS),
        "randomAbbreviation" => chars(3, b"ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
        "randomIP" => format!("{}.{}.{}.{}", r.gen_range(1..255), r.gen_range(0..255), r.gen_range(0..255), r.gen_range(1..255)),
        "randomIPV6" => (0..8).map(|_| format!("{:x}", r.gen_range(0..0xffffu32))).collect::<Vec<_>>().join(":"),
        "randomMACAddress" => (0..6).map(|_| format!("{:02x}", r.gen_range(0..256u32))).collect::<Vec<_>>().join(":"),
        "randomUserAgent" => "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36".into(),
        "randomProtocol" => pick(&["http", "https"]),
        "randomSemver" => format!("{}.{}.{}", r.gen_range(0..10), r.gen_range(0..10), r.gen_range(0..10)),
        "randomFirstName" => pick(FIRST),
        "randomLastName" => pick(LAST),
        "randomFullName" => format!("{} {}", pick(FIRST), pick(LAST)),
        "randomNamePrefix" => pick(&["Mr.", "Mrs.", "Ms.", "Dr."]),
        "randomNameSuffix" => pick(&["Jr.", "Sr.", "II", "PhD", "MD"]),
        "randomJobTitle" => format!("Senior {}", pick(JOBS)),
        "randomJobType" => pick(JOBS),
        "randomUserName" => format!("{}.{}{}", pick(FIRST), pick(LAST), r.gen_range(1..99)).to_lowercase(),
        "randomEmail" | "randomExampleEmail" => {
            format!("{}.{}{}@example.{}", pick(FIRST), pick(LAST), r.gen_range(1..99), pick(TLDS)).to_lowercase()
        }
        "randomDomainName" => format!("{}.{}", pick(WORDS), pick(TLDS)),
        "randomDomainSuffix" => pick(TLDS),
        "randomDomainWord" | "randomWord" | "randomNoun" | "randomLoremWord" => pick(WORDS),
        "randomWords" | "randomLoremWords" => (0..3).map(|_| pick(WORDS)).collect::<Vec<_>>().join(" "),
        "randomUrl" => format!("https://{}.{}", pick(WORDS), pick(TLDS)),
        "randomPhoneNumber" => format!("{}-{}-{}", r.gen_range(200..999), r.gen_range(200..999), r.gen_range(1000..9999)),
        "randomCity" => pick(CITIES),
        "randomCountry" => pick(COUNTRIES),
        "randomCountryCode" => pick(&["DE", "FR", "JP", "BR", "CA", "KE", "IN", "NO", "CL", "AU"]),
        "randomStreetName" => format!("{} Street", pick(LAST)),
        "randomStreetAddress" => format!("{} {} Street", r.gen_range(1..9999), pick(LAST)),
        "randomLatitude" => format!("{:.4}", r.gen_range(-90.0..90.0f64)),
        "randomLongitude" => format!("{:.4}", r.gen_range(-180.0..180.0f64)),
        "randomPrice" => format!("{:.2}", r.gen_range(0.0..1000.0f64)),
        "randomCurrencyCode" => pick(&["USD", "EUR", "JPY", "GBP", "CNY", "INR"]),
        "randomBankAccount" => chars(8, b"0123456789"),
        "randomCreditCardMask" => chars(4, b"0123456789"),
        "randomCompanyName" => format!("{} {}", pick(LAST), pick(&["Inc", "LLC", "Group", "and Sons"])),
        "randomProduct" | "randomProductName" => format!("{} {}", pick(COLORS), pick(WORDS)),
        "randomDateFuture" => (now + chrono::Duration::days(r.gen_range(1..365))).to_rfc2822(),
        "randomDatePast" => (now - chrono::Duration::days(r.gen_range(1..365))).to_rfc2822(),
        "randomDateRecent" => (now - chrono::Duration::hours(r.gen_range(1..24))).to_rfc2822(),
        "randomWeekday" => pick(&["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]),
        "randomMonth" => pick(&["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"]),
        "randomLoremSentence" => {
            let mut s = (0..8).map(|_| pick(LOREM)).collect::<Vec<_>>().join(" ");
            s.push('.');
            s
        }
        "randomLoremParagraph" | "randomLoremText" => (0..4)
            .map(|_| format!("{}.", (0..8).map(|_| pick(LOREM)).collect::<Vec<_>>().join(" ")))
            .collect::<Vec<_>>()
            .join(" "),
        "randomFileName" => format!("{}.{}", pick(WORDS), pick(&["txt", "pdf", "png", "json"])),
        "randomFileExt" => pick(&["txt", "pdf", "png", "json"]),
        "randomMimeType" => pick(&["application/json", "text/plain", "image/png", "application/pdf"]),
        "randomImageUrl" => format!("https://picsum.photos/{}/{}", 640, 480),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(p: &[(&str, &str)]) -> VarMap {
        p.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn precedence_and_nesting() {
        let s = Scopes {
            globals: map(&[("a", "g"), ("g", "G")]),
            collection: map(&[("a", "c"), ("c", "C")]),
            environment: map(&[("a", "e"), ("url", "http://{{host}}/{{c}}"), ("host", "h")]),
            data: map(&[("a", "d")]),
            local: map(&[]),
        };
        assert_eq!(s.resolve("{{a}}-{{g}}-{{ c }}"), "d-G-C");
        assert_eq!(s.resolve("{{url}}/x"), "http://h/C/x");
        assert_eq!(s.resolve("{{missing}} {{a"), "{{missing}} {{a");
        assert_eq!(s.unresolved("{{missing}}{{a}}"), ["missing"]);
    }

    #[test]
    fn self_reference_terminates() {
        let s = Scopes { globals: map(&[("x", "{{x}}!")]), ..Default::default() };
        assert!(s.resolve("{{x}}").ends_with('!'));
    }

    #[test]
    fn dynamics() {
        let s = Scopes::default();
        assert_eq!(s.resolve("{{$guid}}").len(), 36);
        assert!(s.resolve("{{$timestamp}}").parse::<i64>().unwrap() > 1_600_000_000);
        assert!(s.resolve("{{$randomInt}}").parse::<i64>().unwrap() <= 1000);
        assert!(s.resolve("{{$randomEmail}}").contains('@'));
        assert_eq!(s.resolve("{{$nope}}"), "{{$nope}}");
    }
}
