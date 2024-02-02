//! Helpers for working with HTTP requests and responses.
//!
//! These wrap
//! [`genio::Read`](https://docs.rs/genio/0.2.1/genio/trait.Read.html) streams
//! to parse out HTTP headers and then stream the rest of the HTTP body.
//!
//! Uses the [`httparse`](https://docs.rs/httparse/) crate.
//!
//! Compile with the `http` feature to get this module.

mod http_request_reader;
mod http_response_reader;

pub use http_request_reader::{HttpMethod, HttpRequestReader};
pub use http_response_reader::HttpResponseReader;

use httparse::Error as HttpParseError;

#[derive(Debug)]
pub enum Error<RE> {
    /// The underlying
    /// [`genio::Read`](https://docs.rs/genio/0.2.1/genio/trait.Read.html) ran
    /// out of data before a header was fully parsed.
    UnexpectedEof,
    /// We exceeded the underlying buffer (MAX_HEAD_LENGTH) before a header was
    /// fully parsed.
    HeaderBufferFull,
    /// The request had an unexpected HTTP method.
    UnknownHttpMethod,
    /// The `read` method was called before the header was completely parsed.
    ReadBeforeHeadParsed,
    /// There was an error parsing the header.
    HttpParseError(HttpParseError),
    /// There was an I/O error reading from the underlying `genio::Read`.
    ReadError(RE),
}
