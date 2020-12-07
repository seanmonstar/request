//! HTTP Cookies

use std::convert::TryInto;

use crate::header;
use std::fmt;
use std::time::SystemTime;
use std::collections::HashMap;
use url::Url;

/// A single HTTP cookie.
pub struct Cookie<'a>(cookie_crate::Cookie<'a>);

/// A single owned HTTP cookie.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OwnedCookie {
    /// The name of the cookie.
    pub name: String,
    /// The value of the cookie.
    pub value: String,
    /// Is true if the 'HttpOnly' directive is enabled.
    pub http_only: bool,
    /// Is true if the 'Secure' directive is enabled.
    pub secure: bool,
    /// Is true if 'SameSite' directive is 'Lax'.
    pub same_site_lax: bool,
    /// Is true if 'SameSite' directive is 'Strict'.
    pub same_site_strict: bool,
    /// The path directive of the cookie, if set.
    pub path: Option<String>,
    /// The domain directive of the cookie, if set.
    pub domain: Option<String>,
    /// The Max-Age information, if set.
    pub max_age: Option<std::time::Duration>,
    /// The cookie expiration time, if set.
    pub expires: Option<SystemTime>,
}

impl<'a> Cookie<'a> {
    fn parse(value: &'a crate::header::HeaderValue) -> Result<Cookie<'a>, CookieParseError> {
        std::str::from_utf8(value.as_bytes())
            .map_err(cookie_crate::ParseError::from)
            .and_then(cookie_crate::Cookie::parse)
            .map_err(CookieParseError)
            .map(Cookie)
    }

    pub(crate) fn into_inner(self) -> cookie_crate::Cookie<'a> {
        self.0
    }

    /// The name of the cookie.
    pub fn name(&self) -> &str {
        self.0.name()
    }

    /// The value of the cookie.
    pub fn value(&self) -> &str {
        self.0.value()
    }

    /// Returns true if the 'HttpOnly' directive is enabled.
    pub fn http_only(&self) -> bool {
        self.0.http_only().unwrap_or(false)
    }

    /// Returns true if the 'Secure' directive is enabled.
    pub fn secure(&self) -> bool {
        self.0.secure().unwrap_or(false)
    }

    /// Returns true if  'SameSite' directive is 'Lax'.
    pub fn same_site_lax(&self) -> bool {
        self.0.same_site() == Some(cookie_crate::SameSite::Lax)
    }

    /// Returns true if  'SameSite' directive is 'Strict'.
    pub fn same_site_strict(&self) -> bool {
        self.0.same_site() == Some(cookie_crate::SameSite::Strict)
    }

    /// Returns the path directive of the cookie, if set.
    pub fn path(&self) -> Option<&str> {
        self.0.path()
    }

    /// Returns the domain directive of the cookie, if set.
    pub fn domain(&self) -> Option<&str> {
        self.0.domain()
    }

    /// Get the Max-Age information.
    pub fn max_age(&self) -> Option<std::time::Duration> {
        self.0
            .max_age()
            .map(|d| d.try_into().expect("time::Duration into std::time::Duration"))
    }

    /// The cookie expiration time.
    pub fn expires(&self) -> Option<SystemTime> {
        self.0.expires().map(SystemTime::from)
    }
}

impl<'a> Cookie<'a> {

    fn to_owned_cookie(&self) -> OwnedCookie {
        OwnedCookie {
            name: self.name().to_string(),
            value: self.value().to_string(),
            http_only: self.http_only(),
            secure: self.secure(),
            same_site_lax: self.same_site_lax(),
            same_site_strict: self.same_site_strict(),
            path: self.path().map(str::to_string),
            domain: self.domain().map(str::to_string),
            max_age: self.max_age(),
            expires: self.expires(),
        }
    }
}

impl<'a> fmt::Debug for Cookie<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

pub(crate) fn extract_response_cookies<'a>(
    headers: &'a hyper::HeaderMap,
) -> impl Iterator<Item = Result<Cookie<'a>, CookieParseError>> + 'a {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| Cookie::parse(value))
}

/// A persistent cookie store that provides session support.
#[derive(Default)]
pub(crate) struct CookieStore(pub(crate) cookie_store::CookieStore);

impl CookieStore {
    pub(crate) fn exposed<'a>(&'a self, url: &Url) -> HashMap<String, OwnedCookie> {
        self.0.get_request_cookies(url)
            .map(|cookie| (cookie.name().to_string(), Cookie(cookie.to_owned()).to_owned_cookie()))
            .collect()
    }
}

impl<'a> fmt::Debug for CookieStore {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Error representing a parse failure of a 'Set-Cookie' header.
pub(crate) struct CookieParseError(cookie_crate::ParseError);

impl<'a> fmt::Debug for CookieParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'a> fmt::Display for CookieParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for CookieParseError {}
