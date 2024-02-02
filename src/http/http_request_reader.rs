use core::fmt;

use genio::{Read, ReadOverwrite};
use httparse::{Header, Request, Status as HttpParseStatus, EMPTY_HEADER};
use nb;

use crate::http::Error;

pub const MAX_HEAD_LENGTH: usize = 8_000;
pub const MAX_HEADERS: usize = 32;

static EMPTY_STRING: &str = "";

#[derive(Debug, Copy, Clone)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Connect,
    Options,
    Trace,
    Patch,
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            HttpMethod::Get => write!(f, "GET"),
            HttpMethod::Post => write!(f, "POST"),
            HttpMethod::Put => write!(f, "PUT"),
            HttpMethod::Delete => write!(f, "DELETE"),
            HttpMethod::Head => write!(f, "HEAD"),
            HttpMethod::Connect => write!(f, "CONNECT"),
            HttpMethod::Options => write!(f, "OPTIONS"),
            HttpMethod::Trace => write!(f, "TRACE"),
            HttpMethod::Patch => write!(f, "PATCH"),
        }
    }
}

impl HttpMethod {
    fn from_str<RE: core::fmt::Debug>(method: &str) -> Result<HttpMethod, Error<RE>> {
        match method {
            "GET" => Ok(HttpMethod::Get),
            "POST" => Ok(HttpMethod::Post),
            "PUT" => Ok(HttpMethod::Put),
            "DELETE" => Ok(HttpMethod::Delete),
            "HEAD" => Ok(HttpMethod::Head),
            "CONNECT" => Ok(HttpMethod::Connect),
            "OPTIONS" => Ok(HttpMethod::Options),
            "TRACE" => Ok(HttpMethod::Trace),
            "PATCH" => Ok(HttpMethod::Patch),
            _ => Err(Error::UnknownHttpMethod),
        }
    }
}

/// Parsed version of an HTTP request’s head.
///
/// Maintains references into an
/// [`HttpRequestReader`](struct.HttpRequestReader.html)’s buffer for the actual
/// bytes of the strings.
pub struct HttpRequestHead<'buf> {
    pub method: HttpMethod,
    pub path: &'buf str,
    pub version: u8,
    pub headers: [Header<'buf>; MAX_HEADERS],
}

impl<'buf> HttpRequestHead<'buf> {
    fn new() -> Self {
        HttpRequestHead {
            headers: [EMPTY_HEADER; MAX_HEADERS],
            version: 0,
            method: HttpMethod::Get,
            path: EMPTY_STRING,
        }
    }

    /// Returns the value of the header with the given name (ignoring case, per
    /// HTTP spec), or `None` if none is found.
    pub fn header<'me, 'name>(&'me self, name: &'name str) -> Option<&'me [u8]> {
        for h in self.headers.iter() {
            if name.eq_ignore_ascii_case(h.name) {
                return Some(h.value);
            }
        }

        None
    }
}

/// Wraps a [`genio::Read`](https://docs.rs/genio/0.2.1/genio/trait.Read.html)
/// and parses out the HTTP request head into a
/// [`HttpRequestHead`](struct.HttpRequestHead.html), then becomes a
/// `genio::Read` for the body of the request.
///
/// Can handle requests with a maximum of 8K of headers.
///
/// To use, create with [`from_read`](#method.from_read). Then call
/// [`read_head`](#method.read_head) using [`nb::block!`](nb::block!) until it
/// succeeds with an `HttpRequestHead`. Once has, use [`read`](#method.read) to
/// stream the rest of the request.
pub struct HttpRequestReader<R: Read<ReadError = nb::Error<RE>>, RE: core::fmt::Debug> {
    // We buffer some of the data from the reader so that we have a complete
    // picture of the head. Any amount of the buffer not parsed into the head
    // will be streamed out in read() before going back to the underlying
    // reader.
    buf: [u8; MAX_HEAD_LENGTH],
    // Used to keep track of how much of the buffer we haven’t returned when we
    // start streaming it out through read().
    buf_start: usize,
    buf_used: usize,

    // If true, we know that buf contains a valid HTTP head.
    found_head: bool,
    in_reader: R,
}

impl<R: Read<ReadError = nb::Error<RE>>, RE: core::fmt::Debug> HttpRequestReader<R, RE> {
    pub fn from_read(in_reader: R) -> Self {
        HttpRequestReader {
            buf: [0u8; MAX_HEAD_LENGTH],
            buf_used: 0,
            buf_start: 0,
            found_head: false,
            in_reader,
        }
    }

    /// Reads from the underlying reader to try to get a
    /// [`HttpRequestHead`](struct.HttpRequestHead.html).
    ///
    /// Stores data in an internal buffer until it contains a complete HTTP
    /// head. After this method returns an `HttpRequestHead`, use
    /// [`read`](#method.read) to read the rest of the request.
    ///
    /// Errors with [`nb:Error::WouldBlock`](nb:Error::WouldBlock) if the
    /// underlying `genio::Read` would block, or if there currently isn’t a
    /// complete head in the buffer.
    pub fn read_head(&mut self) -> Result<HttpRequestHead, nb::Error<Error<RE>>> {
        if self.found_head {
            // It’s safe to do this because if check() hadn’t succeeded before,
            // we wouldn’t have set found_head to be true.
            return Ok(self.check().unwrap().unwrap().0);
        } else if self.buf_used == MAX_HEAD_LENGTH {
            return Err(nb::Error::Other(Error::HeaderBufferFull));
        }

        let read_amt = match self.in_reader.read(&mut self.buf[self.buf_used..]) {
            Ok(0) => Err(nb::Error::Other(Error::UnexpectedEof)),
            Ok(read_amt) => Ok(read_amt),
            Err(nb::Error::WouldBlock) => Err(nb::Error::WouldBlock),
            Err(nb::Error::Other(err)) => Err(nb::Error::Other(Error::ReadError(err))),
        }?;

        self.buf_used += read_amt;

        return match self.check() {
            Ok(Some((_, parsed_len))) => {
                self.buf_start = parsed_len;
                self.found_head = true;

                // We have to parse a second time because the HttpRequestHead is
                // holding a borrow on self, which keeps us from being able to
                // update buf_start and found_head above.
                Ok(self.check().unwrap().unwrap().0)
            }
            Ok(None) => Err(nb::Error::WouldBlock),
            Err(err) => Err(nb::Error::Other(err)),
        };
    }

    /// Helper to parse our buf into an `HttpRequestHead` struct.
    ///
    /// We can’t cache `HttpRequestHead` in the struct because it has internal
    /// pointers, preventing the struct from being moved in memory.
    fn check(&self) -> Result<Option<(HttpRequestHead, usize)>, Error<RE>> {
        let mut result = HttpRequestHead::new();
        let mut response = Request::new(&mut result.headers);

        match response.parse(&self.buf[0..self.buf_used]) {
            Ok(HttpParseStatus::Complete(parsed_len)) => {
                result.version = response.version.unwrap();
                result.method = HttpMethod::from_str(response.method.unwrap())?;
                result.path = response.path.unwrap();

                return Ok(Some((result, parsed_len)));
            }
            Ok(HttpParseStatus::Partial) => return Ok(None),
            Err(err) => return Err(Error::HttpParseError(err)),
        }
    }

    /// Consumes self to return the underlying `genio::Read`.
    pub fn free(self) -> R {
        self.in_reader
    }
}

impl<R: Read<ReadError = nb::Error<RE>>, RE: core::fmt::Debug> Read for HttpRequestReader<R, RE> {
    type ReadError = nb::Error<Error<RE>>;

    /// Reader for the body of the HTTP request.
    ///
    /// Must be called after [`read_head`](#method.read_head) or else will
    /// return a [`ReadBeforeHeadParsed`](Error::ReadBeforeHeadParsed).
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::ReadError> {
        if !self.found_head {
            return Err(nb::Error::Other(Error::ReadBeforeHeadParsed));
        }

        // This part handles the case where `read_head` read more into its
        // buffer than needed for the headers. We copy out the rest before
        // delegating to our underlying `Read`.
        if self.buf_start < self.buf_used {
            let len = (&self.buf[self.buf_start..self.buf_used])
                .read(buf)
                .unwrap();
            self.buf_start += len;

            Ok(len)
        } else {
            self.in_reader.read(buf).map_err(|err| match err {
                nb::Error::WouldBlock => nb::Error::WouldBlock,
                nb::Error::Other(other_err) => nb::Error::Other(Error::ReadError(other_err)),
            })
        }
    }
}

unsafe impl<R: Read<ReadError = nb::Error<RE>>, RE: core::fmt::Debug> ReadOverwrite
    for HttpRequestReader<R, RE>
{
}
