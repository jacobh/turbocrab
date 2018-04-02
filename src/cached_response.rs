use futures::prelude::*;
use hyper;
use hyper::{Body, Chunk, Headers, Response, Uri};
use std::collections::HashMap;
use std::fs::File;
use tokio_core::reactor::Handle;
use tokio_file_unix;
use tokio_io::AsyncRead;
use tokio_io::codec::BytesCodec;

use cache::url_to_cache_path;

pub struct CachedResponseBuilder {
    url: hyper::Uri,
    status: hyper::StatusCode,
    headers: HashMap<String, Vec<Vec<u8>>>,
}
impl CachedResponseBuilder {
    pub fn new(url: hyper::Uri, status: hyper::StatusCode) -> CachedResponseBuilder {
        CachedResponseBuilder {
            url: url,
            status: status,
            headers: HashMap::new(),
        }
    }
    pub fn with_headers(mut self, headers: &Headers) -> CachedResponseBuilder {
        self.headers = {
            headers
                .iter()
                .map(|header_item| {
                    (
                        header_item.name().to_string(),
                        header_item
                            .raw()
                            .iter()
                            .map(|raw_val| raw_val.to_vec())
                            .collect(),
                    )
                })
                .collect()
        };
        self
    }
    pub fn build(self) -> CachedResponse {
        CachedResponse {
            url: self.url.to_string(),
            status: self.status.as_u16(),
            headers: self.headers,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedResponse {
    url: String,
    status: u16,
    headers: HashMap<String, Vec<Vec<u8>>>,
}
impl CachedResponse {
    fn headers(&self) -> Headers {
        let mut h = Headers::with_capacity(10);
        for (k, vs) in self.headers.clone() {
            for v in vs {
                h.append_raw(k.clone(), v);
            }
        }
        h
    }
    pub fn url(&self) -> Uri {
        self.url.parse().unwrap()
    }
    pub fn cache_path(&self) -> String {
        url_to_cache_path(&self.url())
    }
    pub fn into_response(self, handle: &Handle) -> Response {
        let headers = self.headers();
        let CachedResponse { status, .. } = self;

        let f = File::open(self.cache_path()).unwrap();
        let chunks_stream = tokio_file_unix::File::new_nb(f)
            .unwrap()
            .into_io(handle)
            .unwrap()
            .framed(BytesCodec::new())
            .map(|bytes| -> Result<Chunk, hyper::Error> { Ok(Chunk::from(bytes.freeze())) })
            .map_err(|_| unreachable!());

        let (sender, body) = Body::pair();

        handle.spawn(sender.send_all(chunks_stream).map(|_| ()).map_err(|_| ()));

        Response::new()
            .with_status(hyper::StatusCode::try_from(status).unwrap())
            .with_headers(headers)
            .with_body(body)
    }
}
