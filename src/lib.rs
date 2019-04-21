//! Types and traits giving an interface between low-level http server implementations
//! and services that use them. The interface is based on the `std::futures` API.
//! #![feature(futures_api, async_await, await_macro, existential_type)]
//! 
//! ## Example
//! ```no_run
//! use futures::{
//!     future::{self, FutureObj},
//! };
//! use http_service::{HttpService, Response};
//! use std::net::{IpAddr, Ipv4Addr, SocketAddr};
//! 
//! struct Server {
//!     message: Vec<u8>,
//! }
//! 
//! impl Server {
//!     fn create(message: Vec<u8>) -> Server {
//!         Server {
//!             message,
//!         }
//!     }
//! 
//!     pub fn serve(s: Server) {
//!         let a = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
//!         http_service_hyper::serve(s, a);
//!     }
//! }
//! 
//! impl HttpService for Server {
//!     type Connection = ();
//!    type ConnectionFuture = future::Ready<Result<(), std::io::Error>>;
//!     type Fut = FutureObj<'static, Result<http_service::Response, std::io::Error>>;
//!     
//!     fn connect(&self) -> Self::ConnectionFuture {
//!         future::ok(())
//!     }
//! 
//!     fn respond(&self, _conn: &mut (), _req: http_service::Request) -> Self::Fut {
//!         let message = self.message.clone();
//!         FutureObj::new(Box::new(
//!             async move {
//!                 Ok(Response::new(http_service::Body::from(message)))
//!             }
//!         ))
//!     }
//! }
//! 
//! fn main() {
//!     let s = Server::create(String::from("Hello, World").into_bytes());
//!     Server::serve(s);
//! }
//! ```

#![forbid(future_incompatible, rust_2018_idioms)]
#![deny(missing_debug_implementations, nonstandard_style)]
#![warn(missing_docs, missing_doc_code_examples)]
#![cfg_attr(test, deny(warnings))]
#![feature(futures_api, async_await, await_macro, arbitrary_self_types)]

use bytes::Bytes;
use futures::{
    future,
    prelude::*,
    stream::{self, StreamObj},
    task::Context,
    Poll,
};

use std::marker::Unpin;
use std::pin::Pin;

/// The raw body of an http request or response.
///
/// A body is a stream of `Bytes` values, which are shared handles to byte buffers.
/// Both `Body` and `Bytes` values can be easily created from standard owned byte buffer types
/// like `Vec<u8>` or `String`, using the `From` trait.
#[derive(Debug)]
pub struct Body {
    stream: StreamObj<'static, Result<Bytes, std::io::Error>>,
}

impl Body {
    /// Create an empty body.
    pub fn empty() -> Self {
        Body::from_stream(stream::empty())
    }

    /// Create a body from a stream of `Bytes`
    pub fn from_stream<S>(s: S) -> Self
    where
        S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    {
        Self {
            stream: StreamObj::new(Box::new(s)),
        }
    }

    /// Reads the stream into a new `Vec`.
    pub async fn into_vec(mut self) -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        while let Some(chunk) = await!(self.next()) {
            bytes.extend(chunk?);
        }
        Ok(bytes)
    }
}

impl<T: Into<Bytes> + Send> From<T> for Body {
    fn from(x: T) -> Self {
        Self::from_stream(stream::once(future::ok(x.into())))
    }
}

impl Unpin for Body {}

impl Stream for Body {
    type Item = Result<Bytes, std::io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.stream).poll_next(cx)
    }
}

/// An HTTP request with a streaming body.
pub type Request = http::Request<Body>;

/// An HTTP response with a streaming body.
pub type Response = http::Response<Body>;

/// An async HTTP service
///
/// An instance represents a service as a whole. The associated `Conn` type
/// represents a particular connection, and may carry connection-specific state.
pub trait HttpService: Send + Sync + 'static {
    /// An individual connection.
    ///
    /// This associated type is used to establish and hold any per-connection state
    /// needed by the service.
    type Connection: Send + 'static;

    /// A future for setting up an individual connection.
    ///
    /// This method is called each time the server receives a new connection request,
    /// but before actually exchanging any data with the client.
    ///
    /// Returning an error will result in the server immediately dropping
    /// the connection.
    type ConnectionFuture: Send + 'static + TryFuture<Ok = Self::Connection>;

    /// Initiate a new connection.
    ///
    /// This method is given access to the global service (`&self`), which may provide
    /// handles to connection pools, thread pools, or other global data.
    fn connect(&self) -> Self::ConnectionFuture;

    /// The async computation for producing the response.
    ///
    /// Returning an error will result in the server immediately dropping
    /// the connection. It is usually preferable to instead return an HTTP response
    /// with an error status code.
    type Fut: Send + 'static + TryFuture<Ok = Response>;

    /// Begin handling a single request.
    ///
    /// The handler is given shared access to the service itself, and mutable access
    /// to the state for the connection where the request is taking place.
    fn respond(&self, conn: &mut Self::Connection, req: Request) -> Self::Fut;
}

impl<F, Fut> HttpService for F
where
    F: Send + Sync + 'static + Fn(Request) -> Fut,
    Fut: Send + 'static + TryFuture<Ok = Response>,
    Fut::Error: Send,
{
    type Connection = ();
    type ConnectionFuture = future::Ready<Result<(), Fut::Error>>;
    fn connect(&self) -> Self::ConnectionFuture {
        future::ok(())
    }

    type Fut = Fut;
    fn respond(&self, _: &mut (), req: Request) -> Self::Fut {
        (self)(req)
    }
}
