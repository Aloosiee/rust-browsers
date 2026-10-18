//! `browser:` URL scheme serving the built-in pages, such as `browser:home`.

use servo::protocol_handler::{
    DoneChannel, FetchContext, HttpStatus, NetworkError, ProtocolHandler, Request, ResourceFetchTiming, Response,
    ResponseBody,
};
use std::future::Future;
use std::pin::Pin;

const HOME: &str = include_str!("home.html");

/// Serves built-in pages from memory.
pub struct BrowserPages;

impl ProtocolHandler for BrowserPages {
    fn load<'a>(
        &'a self,
        request: &'a mut Request,
        _done_chan: &mut DoneChannel,
        _context: &FetchContext,
    ) -> Pin<Box<dyn Future<Output = Response> + Send + 'a>> {
        let url = request.current_url();
        let body = match url.path() {
            "home" | "newtab" | "" => Some(HOME.as_bytes().to_vec()),
            _ => None,
        };
        let response = match body {
            Some(bytes) => {
                let mut response = Response::new(url, ResourceFetchTiming::new(request.timing_type()));
                *response.body.lock() = ResponseBody::Done(bytes);
                response.headers.insert(
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_static("text/html; charset=utf-8"),
                );
                response.status = HttpStatus::default();
                response
            }
            None => Response::network_error(NetworkError::ResourceLoadError("No such browser page".into())),
        };
        Box::pin(std::future::ready(response))
    }

    fn is_fetchable(&self) -> bool {
        true
    }

    fn is_secure(&self) -> bool {
        true
    }
}
