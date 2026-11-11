//! Persistent cookie jar with RFC 6265 style domain/path matching.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    #[serde(default = "root")]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<DateTime<Utc>>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    /// Set when the cookie had no Domain attribute (exact host match only).
    #[serde(default)]
    pub host_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_site: Option<String>,
}

fn root() -> String {
    "/".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CookieJar {
    #[serde(default)]
    pub cookies: Vec<Cookie>,
}

fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(d) = DateTime::parse_from_rfc2822(s) {
        return Some(d.with_timezone(&Utc));
    }
    let cleaned = s.replace("GMT", "").replace('-', " ");
    let cleaned = cleaned.trim();
    for fmt in ["%a, %d %b %Y %H:%M:%S", "%A, %d %b %y %H:%M:%S", "%a %b %e %H:%M:%S %Y", "%d %b %Y %H:%M:%S"] {
        if let Ok(d) = chrono::NaiveDateTime::parse_from_str(cleaned, fmt) {
            return Some(d.and_utc());
        }
    }
    None
}

impl Cookie {
    /// Parse a `Set-Cookie` header value received from `url`.
    pub fn parse(header: &str, url: &url::Url) -> Option<Cookie> {
        let mut parts = header.split(';');
        let (name, value) = parts.next()?.split_once('=')?;
        let host = url.host_str().unwrap_or("").to_lowercase();
        let default_path = match url.path().rfind('/') {
            Some(0) | None => "/".to_string(),
            Some(i) => url.path()[..i].to_string(),
        };
        let mut c = Cookie {
            name: name.trim().to_string(),
            value: value.trim().trim_matches('"').to_string(),
            domain: host,
            path: default_path,
            host_only: true,
            ..Default::default()
        };
        if c.name.is_empty() {
            return None;
        }
        let mut max_age: Option<i64> = None;
        for attr in parts {
            let (k, v) = match attr.split_once('=') {
                Some((k, v)) => (k.trim().to_lowercase(), v.trim()),
                None => (attr.trim().to_lowercase(), ""),
            };
            match k.as_str() {
                "domain" if !v.is_empty() => {
                    c.domain = v.trim_start_matches('.').to_lowercase();
                    c.host_only = false;
                }
                "path" if v.starts_with('/') => c.path = v.to_string(),
                "expires" => c.expires = parse_date(v),
                "max-age" => max_age = v.parse().ok(),
                "secure" => c.secure = true,
                "httponly" => c.http_only = true,
                "samesite" => c.same_site = Some(v.to_string()),
                _ => {}
            }
        }
        if let Some(s) = max_age {
            c.expires = Some(Utc::now() + chrono::Duration::seconds(s));
        }
        Some(c)
    }

    pub fn expired(&self) -> bool {
        self.expires.map(|e| e <= Utc::now()).unwrap_or(false)
    }

    pub fn matches(&self, url: &url::Url) -> bool {
        let host = url.host_str().unwrap_or("").to_lowercase();
        let domain_ok = if self.host_only {
            host == self.domain
        } else {
            host == self.domain || host.ends_with(&format!(".{}", self.domain))
        };
        let path = url.path();
        let path_ok = path == self.path
            || (path.starts_with(&self.path) && (self.path.ends_with('/') || path[self.path.len()..].starts_with('/')));
        domain_ok && path_ok && !self.expired() && (!self.secure || url.scheme() == "https" || host == "localhost" || host == "127.0.0.1")
    }
}

impl CookieJar {
    pub fn store(&mut self, cookie: Cookie) {
        self.cookies.retain(|c| !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path));
        if !cookie.expired() {
            self.cookies.push(cookie);
        }
    }

    /// Store a `Set-Cookie` header; returns the parsed cookie.
    pub fn store_header(&mut self, header: &str, url: &url::Url) -> Option<Cookie> {
        let c = Cookie::parse(header, url)?;
        self.store(c.clone());
        Some(c)
    }

    pub fn matching(&self, url: &url::Url) -> Vec<&Cookie> {
        let mut v: Vec<&Cookie> = self.cookies.iter().filter(|c| c.matches(url)).collect();
        v.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
        v
    }

    /// Value for the `Cookie` request header, if any cookie applies.
    pub fn header_for(&self, url: &url::Url) -> Option<String> {
        let m = self.matching(url);
        if m.is_empty() {
            return None;
        }
        Some(m.iter().map(|c| format!("{}={}", c.name, c.value)).collect::<Vec<_>>().join("; "))
    }

    pub fn purge_expired(&mut self) {
        self.cookies.retain(|c| !c.expired());
    }

    pub fn remove(&mut self, domain: &str, name: Option<&str>) -> usize {
        let n = self.cookies.len();
        self.cookies.retain(|c| !(c.domain == domain && name.map(|x| x == c.name).unwrap_or(true)));
        n - self.cookies.len()
    }

    pub fn domains(&self) -> Vec<String> {
        let mut d: Vec<String> = self.cookies.iter().map(|c| c.domain.clone()).collect();
        d.sort();
        d.dedup();
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_matches() {
        let u = url::Url::parse("http://api.example.com/v1/login").unwrap();
        let mut jar = CookieJar::default();
        jar.store_header("sid=abc; Path=/; HttpOnly", &u).unwrap();
        jar.store_header("wide=1; Domain=.example.com; Max-Age=60", &u).unwrap();
        jar.store_header("deep=1; Path=/v1/admin", &u).unwrap();
        jar.store_header("old=1; Expires=Wed, 21 Oct 2015 07:28:00 GMT", &u).unwrap();
        assert_eq!(jar.cookies.len(), 3);
        assert_eq!(jar.header_for(&u).unwrap(), "wide=1; sid=abc");
        let other = url::Url::parse("http://www.example.com/v1/admin/x").unwrap();
        assert_eq!(jar.header_for(&other).unwrap(), "wide=1");
        let admin = url::Url::parse("http://api.example.com/v1/admin/x").unwrap();
        assert!(jar.header_for(&admin).unwrap().starts_with("deep=1"));
        // overwrite + delete through expiry
        jar.store_header("sid=zzz; Path=/", &u);
        assert!(jar.header_for(&u).unwrap().contains("sid=zzz"));
        jar.store_header("sid=; Path=/; Max-Age=0", &u);
        assert!(!jar.header_for(&u).unwrap().contains("sid"));
        assert_eq!(jar.remove("example.com", None), 1);
    }
}
