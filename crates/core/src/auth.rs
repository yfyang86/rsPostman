//! Authorization helpers: Basic, Bearer, API key, Digest, OAuth 1.0a, OAuth 2.0,
//! AWS Signature v4 and JWT bearer.

use crate::http::{basic_token, Prepared};
use crate::model::Auth;
use crate::vars::Scopes;
use anyhow::{anyhow, bail, Result};
use base64::Engine;
use hmac::{Hmac, Mac};
use md5::Md5;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};

pub const AUTH_TYPES: &[&str] = &["inherit", "noauth", "basic", "bearer", "apikey", "digest", "oauth1", "oauth2", "awsv4", "jwt"];

const RFC3986: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

fn enc(s: &str) -> String {
    utf8_percent_encode(s, RFC3986).to_string()
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("hmac key");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

/// Apply `auth` to the prepared request. Unknown types are an error so that a
/// request is never silently sent unauthenticated.
pub fn apply(auth: &Auth, p: &mut Prepared, sc: &Scopes) -> Result<()> {
    let g = |k: &str| sc.resolve(auth.get(k));
    match auth.kind.as_str() {
        "" | "noauth" | "inherit" => {}
        "basic" => p.set_header("Authorization", &format!("Basic {}", basic_token(&g("username"), &g("password")))),
        "bearer" => {
            let t = g("token");
            if !t.is_empty() {
                p.set_header("Authorization", &format!("Bearer {t}"));
            }
        }
        "apikey" => {
            let (k, v) = (g("key"), g("value"));
            if !k.is_empty() {
                if g("in") == "query" {
                    p.add_query(&k, &v);
                } else {
                    p.set_header(&k, &v);
                }
            }
        }
        "oauth2" => {
            let t = g("accessToken");
            if !t.is_empty() {
                if g("addTokenTo") == "queryParams" {
                    p.add_query("access_token", &t);
                } else {
                    let prefix = match auth.params.get("headerPrefix") {
                        Some(x) => sc.resolve(x),
                        None => "Bearer".into(),
                    };
                    p.set_header("Authorization", format!("{} {t}", prefix.trim()).trim());
                }
            }
        }
        "digest" => p.digest = Some((g("username"), g("password"))),
        "awsv4" => {
            let now = chrono::Utc::now();
            let body = p.body.bytes()?.unwrap_or_default();
            let url = url::Url::parse(&p.url).map_err(|e| anyhow!("invalid URL for AWS signature: {e}"))?;
            let host = match url.port() {
                Some(port) => format!("{}:{port}", url.host_str().unwrap_or("")),
                None => url.host_str().unwrap_or("").to_string(),
            };
            let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
            let payload_hash = hex::encode(Sha256::digest(&body));
            let mut sign = vec![("host".to_string(), host), ("x-amz-date".to_string(), amz_date.clone())];
            p.set_header("X-Amz-Date", &amz_date);
            let service = if g("service").is_empty() { "execute-api".to_string() } else { g("service") };
            if service == "s3" {
                sign.push(("x-amz-content-sha256".into(), payload_hash.clone()));
                p.set_header("X-Amz-Content-Sha256", &payload_hash);
            }
            let token = g("sessionToken");
            if !token.is_empty() {
                sign.push(("x-amz-security-token".into(), token.clone()));
                p.set_header("X-Amz-Security-Token", &token);
            }
            if let Some(ct) = p.header("content-type") {
                sign.push(("content-type".into(), ct.to_string()));
            }
            let region = if g("region").is_empty() { "us-east-1".to_string() } else { g("region") };
            let value = sigv4(&p.method, &url, &sign, &payload_hash, &g("accessKey"), &g("secretKey"), &region, &service, &amz_date);
            p.set_header("Authorization", &value);
        }
        "oauth1" => oauth1(auth, p, sc)?,
        "jwt" => {
            let token = jwt(&g("algorithm"), &g("secret"), g("isSecretBase64Encoded") == "true", &g("payload"), &g("header"))?;
            if g("addTokenTo") == "queryParam" {
                let name = if g("queryParamKey").is_empty() { "token".to_string() } else { g("queryParamKey") };
                p.add_query(&name, &token);
            } else {
                let prefix = match auth.params.get("headerPrefix") {
                    Some(x) => sc.resolve(x),
                    None => "Bearer".into(),
                };
                p.set_header("Authorization", format!("{} {token}", prefix.trim()).trim());
            }
        }
        other => bail!("unsupported auth type `{other}` (supported: {})", AUTH_TYPES.join(", ")),
    }
    Ok(())
}

fn h<D: Digest>(s: &str) -> String {
    hex::encode(D::digest(s.as_bytes()))
}

pub(crate) fn challenge_params(ch: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let s = ch.trim();
    let s = if s.len() >= 6 { &s[6..] } else { "" };
    let (mut key, mut val, mut in_val, mut quoted) = (String::new(), String::new(), false, false);
    for c in s.chars().chain(std::iter::once(',')) {
        if !in_val {
            if c == '=' {
                in_val = true;
            } else if c != ',' {
                key.push(c);
            }
        } else if c == '"' {
            quoted = !quoted;
        } else if c == ',' && !quoted {
            out.insert(key.trim().to_lowercase(), val.trim().to_string());
            key.clear();
            val.clear();
            in_val = false;
        } else {
            val.push(c);
        }
    }
    out
}

/// Build the `Authorization: Digest ...` value answering a `WWW-Authenticate` challenge (RFC 7616).
pub fn digest_header(challenge: &str, user: &str, pass: &str, method: &str, uri: &str, body: &[u8]) -> String {
    digest_header_with(challenge, user, pass, method, uri, body, &hex::encode(rand::random::<[u8; 8]>()))
}

fn digest_header_with(challenge: &str, user: &str, pass: &str, method: &str, uri: &str, body: &[u8], cnonce: &str) -> String {
    let c = challenge_params(challenge);
    let get = |k: &str| c.get(k).cloned().unwrap_or_default();
    let (realm, nonce, opaque) = (get("realm"), get("nonce"), get("opaque"));
    let algorithm = if get("algorithm").is_empty() { "MD5".to_string() } else { get("algorithm") };
    let sha = algorithm.to_uppercase().starts_with("SHA-256");
    let hash = |s: &str| if sha { h::<Sha256>(s) } else { h::<Md5>(s) };
    let qops = get("qop");
    let qop = if qops.split(',').any(|q| q.trim() == "auth") {
        "auth"
    } else if qops.split(',').any(|q| q.trim() == "auth-int") {
        "auth-int"
    } else {
        ""
    };
    let nc = "00000001";
    let mut ha1 = hash(&format!("{user}:{realm}:{pass}"));
    if algorithm.to_lowercase().ends_with("-sess") {
        ha1 = hash(&format!("{ha1}:{nonce}:{cnonce}"));
    }
    let ha2 = if qop == "auth-int" {
        let hb = if sha { hex::encode(Sha256::digest(body)) } else { hex::encode(Md5::digest(body)) };
        hash(&format!("{method}:{uri}:{hb}"))
    } else {
        hash(&format!("{method}:{uri}"))
    };
    let response = if qop.is_empty() {
        hash(&format!("{ha1}:{nonce}:{ha2}"))
    } else {
        hash(&format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}"))
    };
    let mut v = format!(
        "Digest username=\"{user}\", realm=\"{realm}\", nonce=\"{nonce}\", uri=\"{uri}\", algorithm={algorithm}, response=\"{response}\""
    );
    if !qop.is_empty() {
        v.push_str(&format!(", qop={qop}, nc={nc}, cnonce=\"{cnonce}\""));
    }
    if !opaque.is_empty() {
        v.push_str(&format!(", opaque=\"{opaque}\""));
    }
    v
}

/// AWS Signature Version 4 `Authorization` header value.
#[allow(clippy::too_many_arguments)]
pub fn sigv4(
    method: &str,
    url: &url::Url,
    headers: &[(String, String)],
    payload_hash: &str,
    access_key: &str,
    secret_key: &str,
    region: &str,
    service: &str,
    amz_date: &str,
) -> String {
    let date = &amz_date[..8.min(amz_date.len())];
    let mut hs: Vec<(String, String)> =
        headers.iter().map(|(k, v)| (k.to_lowercase(), v.split_whitespace().collect::<Vec<_>>().join(" "))).collect();
    hs.sort();
    let canonical_headers: String = hs.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed: String = hs.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let mut q: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (enc(&k), enc(&v))).collect();
    q.sort();
    let canonical_query = q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let decoded_path = percent_encoding::percent_decode_str(url.path()).decode_utf8_lossy().to_string();
    let path = decoded_path.split('/').map(enc).collect::<Vec<_>>().join("/");
    let path = if path.is_empty() { "/".to_string() } else { path };
    let canonical = format!("{method}\n{path}\n{canonical_query}\n{canonical_headers}\n{signed}\n{payload_hash}");
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", hex::encode(Sha256::digest(canonical.as_bytes())));
    let k = hmac_sha256(format!("AWS4{secret_key}").as_bytes(), date.as_bytes());
    let k = hmac_sha256(&k, region.as_bytes());
    let k = hmac_sha256(&k, service.as_bytes());
    let k = hmac_sha256(&k, b"aws4_request");
    let signature = hex::encode(hmac_sha256(&k, to_sign.as_bytes()));
    format!("AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed}, Signature={signature}")
}

/// OAuth 1.0a signature base string and signature (RFC 5849).
pub fn oauth1_signature(
    method: &str,
    url: &url::Url,
    params: &[(String, String)],
    signature_method: &str,
    consumer_secret: &str,
    token_secret: &str,
) -> String {
    let mut all: Vec<(String, String)> = params.iter().map(|(k, v)| (enc(k), enc(v))).collect();
    all.extend(url.query_pairs().map(|(k, v)| (enc(&k), enc(&v))));
    all.sort();
    let normalized = all.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let mut base_url = url.clone();
    base_url.set_query(None);
    base_url.set_fragment(None);
    let base = format!("{}&{}&{}", method.to_uppercase(), enc(base_url.as_str()), enc(&normalized));
    let key = format!("{}&{}", enc(consumer_secret), enc(token_secret));
    let b64 = base64::engine::general_purpose::STANDARD;
    match signature_method {
        "PLAINTEXT" => key,
        "HMAC-SHA256" => b64.encode(hmac_sha256(key.as_bytes(), base.as_bytes())),
        _ => {
            let mut m = Hmac::<Sha1>::new_from_slice(key.as_bytes()).expect("hmac key");
            m.update(base.as_bytes());
            b64.encode(m.finalize().into_bytes())
        }
    }
}

fn oauth1(auth: &Auth, p: &mut Prepared, sc: &Scopes) -> Result<()> {
    let g = |k: &str| sc.resolve(auth.get(k));
    let or = |v: String, d: String| if v.is_empty() { d } else { v };
    let method = or(g("signatureMethod"), "HMAC-SHA1".into());
    if !["HMAC-SHA1", "HMAC-SHA256", "PLAINTEXT"].contains(&method.as_str()) {
        bail!("unsupported OAuth 1 signature method `{method}`");
    }
    let mut params = vec![
        ("oauth_consumer_key".to_string(), g("consumerKey")),
        ("oauth_signature_method".to_string(), method.clone()),
        ("oauth_timestamp".to_string(), or(g("timestamp"), chrono::Utc::now().timestamp().to_string())),
        ("oauth_nonce".to_string(), or(g("nonce"), hex::encode(rand::random::<[u8; 8]>()))),
        ("oauth_version".to_string(), or(g("version"), "1.0".into())),
    ];
    if !g("token").is_empty() {
        params.push(("oauth_token".into(), g("token")));
    }
    let url = url::Url::parse(&p.url).map_err(|e| anyhow!("invalid URL for OAuth 1: {e}"))?;
    let mut for_sig = params.clone();
    if let crate::http::BodySpec::Urlencoded { fields } = &p.body {
        for_sig.extend(fields.iter().cloned());
    }
    let sig = oauth1_signature(&p.method, &url, &for_sig, &method, &g("consumerSecret"), &g("tokenSecret"));
    params.push(("oauth_signature".into(), sig));
    if g("addParamsToHeader") == "false" {
        for (k, v) in &params {
            p.add_query(k, &enc(v));
        }
    } else {
        let mut parts = vec![];
        if !g("realm").is_empty() {
            parts.push(format!("realm=\"{}\"", g("realm")));
        }
        parts.extend(params.iter().map(|(k, v)| format!("{k}=\"{}\"", enc(v))));
        p.set_header("Authorization", &format!("OAuth {}", parts.join(", ")));
    }
    Ok(())
}

/// Create a signed JWT (HS256 / HS384 / HS512).
pub fn jwt(algorithm: &str, secret: &str, secret_b64: bool, payload: &str, header: &str) -> Result<String> {
    let alg = if algorithm.is_empty() { "HS256" } else { algorithm };
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let key = if secret_b64 {
        base64::engine::general_purpose::STANDARD.decode(secret.trim()).map_err(|e| anyhow!("JWT secret is not valid base64: {e}"))?
    } else {
        secret.as_bytes().to_vec()
    };
    let mut hdr: serde_json::Value =
        if header.trim().is_empty() { serde_json::json!({}) } else { serde_json::from_str(header).map_err(|e| anyhow!("JWT header is not valid JSON: {e}"))? };
    hdr["alg"] = serde_json::json!(alg);
    if hdr.get("typ").is_none() {
        hdr["typ"] = serde_json::json!("JWT");
    }
    let payload: serde_json::Value =
        if payload.trim().is_empty() { serde_json::json!({}) } else { serde_json::from_str(payload).map_err(|e| anyhow!("JWT payload is not valid JSON: {e}"))? };
    let signing = format!("{}.{}", b64.encode(hdr.to_string()), b64.encode(payload.to_string()));
    macro_rules! sign {
        ($d:ty) => {{
            let mut m = Hmac::<$d>::new_from_slice(&key).expect("hmac key");
            m.update(signing.as_bytes());
            b64.encode(m.finalize().into_bytes())
        }};
    }
    let sig = match alg {
        "HS256" => sign!(Sha256),
        "HS384" => sign!(Sha384),
        "HS512" => sign!(Sha512),
        other => bail!("unsupported JWT algorithm `{other}` (supported: HS256, HS384, HS512)"),
    };
    Ok(format!("{signing}.{sig}"))
}

/// Fetch an OAuth 2.0 access token (client credentials, password or refresh token grant).
/// Returns the token endpoint's JSON response.
pub fn oauth2_fetch_token(auth: &Auth, sc: &Scopes, settings: &crate::http::Settings) -> Result<serde_json::Value> {
    let g = |k: &str| sc.resolve(auth.get(k));
    let grant = match g("grant_type").as_str() {
        "" | "client_credentials" => "client_credentials",
        "password_credentials" | "password" => "password",
        "refresh_token" => "refresh_token",
        other => bail!("grant type `{other}` needs a browser; paste the access token instead (supported here: client_credentials, password, refresh_token)"),
    };
    let token_url = g("accessTokenUrl");
    if token_url.is_empty() {
        bail!("OAuth 2: access token URL is empty");
    }
    let mut fields = vec![("grant_type".to_string(), grant.to_string())];
    if grant == "password" {
        fields.push(("username".into(), g("username")));
        fields.push(("password".into(), g("password")));
    }
    if grant == "refresh_token" {
        fields.push(("refresh_token".into(), g("refreshToken")));
    }
    if !g("scope").is_empty() {
        fields.push(("scope".into(), g("scope")));
    }
    let mut p = Prepared { method: "POST".into(), url: token_url, ..Default::default() };
    p.headers.push(("Content-Type".into(), "application/x-www-form-urlencoded".into()));
    p.headers.push(("Accept".into(), "application/json".into()));
    if g("client_authentication") == "body" {
        fields.push(("client_id".into(), g("clientId")));
        fields.push(("client_secret".into(), g("clientSecret")));
    } else {
        p.headers.push(("Authorization".into(), format!("Basic {}", basic_token(&g("clientId"), &g("clientSecret")))));
    }
    p.body = crate::http::BodySpec::Urlencoded { fields };
    let resp = crate::http::execute(&p, settings, None)?;
    let json = resp.json().ok_or_else(|| anyhow!("token endpoint returned {} with a non-JSON body: {}", resp.status, resp.text()))?;
    if resp.status >= 400 || json.get("access_token").is_none() {
        bail!("token endpoint returned {}: {}", resp.status, json);
    }
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigv4_matches_aws_reference_vector() {
        // "GET iam ListUsers" example from the AWS Signature V4 documentation.
        let url = url::Url::parse("https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08").unwrap();
        let headers = vec![
            ("Content-Type".to_string(), "application/x-www-form-urlencoded; charset=utf-8".to_string()),
            ("Host".to_string(), "iam.amazonaws.com".to_string()),
            ("X-Amz-Date".to_string(), "20150830T123600Z".to_string()),
        ];
        let v = sigv4(
            "GET",
            &url,
            &headers,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "iam",
            "20150830T123600Z",
        );
        assert_eq!(
            v,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
        );
    }

    #[test]
    fn digest_matches_rfc2617_example() {
        let ch = r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#;
        let v = digest_header_with(ch, "Mufasa", "Circle Of Life", "GET", "/dir/index.html", b"", "0a4f113b");
        assert!(v.contains("response=\"6629fae49393a05397450978507c4ef1\""), "{v}");
        assert!(v.contains("opaque=\"5ccc069c403ebaf9f0171e9517f40e41\""));
    }

    #[test]
    fn jwt_hs256_known_token() {
        // jwt.io default example
        let t = jwt("HS256", "your-256-bit-secret", false, r#"{"sub":"1234567890","name":"John Doe","iat":1516239022}"#, "").unwrap();
        assert_eq!(
            t,
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"
        );
    }

    #[test]
    fn oauth1_rfc5849_plaintext_and_shape() {
        let url = url::Url::parse("http://example.com/request?b5=%3D%253D&a3=a").unwrap();
        let params = vec![("oauth_consumer_key".to_string(), "9djdj82h48djs9d2".to_string())];
        assert_eq!(oauth1_signature("POST", &url, &params, "PLAINTEXT", "j49sk3j29djd", "dh893hdasih9"), "j49sk3j29djd&dh893hdasih9");
        let s = oauth1_signature("POST", &url, &params, "HMAC-SHA1", "j49sk3j29djd", "dh893hdasih9");
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(s).unwrap().len(), 20);
    }

    #[test]
    fn applies_simple_schemes() {
        let sc = Scopes::default();
        let mut p = Prepared { method: "GET".into(), url: "http://h/x".into(), ..Default::default() };
        apply(&Auth::new("basic", &[("username", "u"), ("password", "p")]), &mut p, &sc).unwrap();
        assert_eq!(p.header("authorization"), Some("Basic dTpw"));
        apply(&Auth::new("apikey", &[("key", "k"), ("value", "v w"), ("in", "query")]), &mut p, &sc).unwrap();
        assert_eq!(p.url, "http://h/x?k=v%20w");
        apply(&Auth::new("oauth2", &[("accessToken", "T")]), &mut p, &sc).unwrap();
        assert_eq!(p.header("authorization"), Some("Bearer T"));
        assert!(apply(&Auth::new("ntlm", &[]), &mut p, &sc).is_err());
    }
}
