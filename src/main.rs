#![feature(conservative_impl_trait)]

// extern crate failure;
extern crate crossbeam_channel;
extern crate evmap;
extern crate futures;
extern crate hyper;
extern crate hyper_tls;
#[macro_use]
extern crate serde_derive;
extern crate tokio_core;
extern crate url;

use std::str::FromStr;
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use futures::prelude::*;

use hyper::{Client, Headers, Uri};
use hyper::server::{Http, Request, Response, Service};

use tokio_core::reactor::{Core, Handle};

struct CachedResponseBuilder {
    url: hyper::Uri,
    status: hyper::StatusCode,
    headers: HashMap<String, Vec<Vec<u8>>>,
    body: Vec<u8>,
}
impl CachedResponseBuilder {
    fn new(url: hyper::Uri, status: hyper::StatusCode) -> CachedResponseBuilder {
        CachedResponseBuilder {
            url: url,
            status: status,
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }
    fn with_body(mut self, body: Vec<u8>) -> CachedResponseBuilder {
        self.body = body;
        self
    }
    fn with_headers(mut self, headers: &Headers) -> CachedResponseBuilder {
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
    fn build(self) -> CachedResponse {
        CachedResponse {
            url: self.url.to_string(),
            status: self.status.as_u16(),
            headers: self.headers,
            body: self.body,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedResponse {
    url: String,
    status: u16,
    headers: HashMap<String, Vec<Vec<u8>>>,
    body: Vec<u8>,
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
}
impl Into<Response> for CachedResponse {
    fn into(self) -> Response {
        let headers = self.headers();
        let CachedResponse { status, body, .. } = self;

        Response::new()
            .with_status(hyper::StatusCode::try_from(status).unwrap())
            .with_headers(headers)
            .with_body(body)
    }
}

struct TurboCache {
    cache_reader: evmap::ReadHandle<Uri, Box<CachedResponse>>,
    cache_sender: crossbeam_channel::Sender<CachedResponse>,
}
impl TurboCache {
    fn new() -> TurboCache {
        let (reader, mut writer) = evmap::new();
        let (tx, rx) = crossbeam_channel::unbounded::<CachedResponse>();

        thread::spawn(move || {
            for cached_response in rx.iter() {
                writer.insert(
                    cached_response.url.parse().unwrap(),
                    Box::new(cached_response),
                );
                writer.refresh();
            }
        });

        TurboCache {
            cache_reader: reader,
            cache_sender: tx,
        }
    }
    fn get(&self, url: &Uri) -> Option<CachedResponse> {
        self.cache_reader.get_and(url, |resps| *(resps[0].clone()))
    }
    fn append_async(&self, resp: CachedResponse) {
        self.cache_sender.send(resp).unwrap()
    }
}

struct TurboCrab {
    client: Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>,
    cache: Arc<TurboCache>,
}
impl TurboCrab {
    fn new(client_handle: &Handle) -> TurboCrab {
        TurboCrab {
            client: Client::configure()
                .connector(hyper_tls::HttpsConnector::new(8, client_handle).unwrap())
                .build(client_handle),
            cache: Arc::new(TurboCache::new()),
        }
    }
    fn get_url(&self, url: Uri) -> Box<Future<Item = Response, Error = hyper::Error> + 'static> {
        let cache = self.cache.clone();

        if let Some(resp) = cache.get(&url) {
            return Box::new(futures::future::ok(resp.into()));
        }

        Box::new(self.client.get(url.clone()).and_then(|resp| {
            let resp_builder =
                CachedResponseBuilder::new(url, resp.status()).with_headers(resp.headers());

            resp.body()
                .concat2()
                .map(|chunk| chunk.to_vec())
                .map(move |body: Vec<u8>| {
                    let cached_response = resp_builder.with_body(body).build();
                    cache.append_async(cached_response.clone());
                    cached_response.into()
                })
        }))
    }
}

impl Service for TurboCrab {
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

    fn call(&self, req: Request) -> Self::Future {
        let source_url: Option<Uri> = req.query().and_then(|s| {
            url::form_urlencoded::parse(s.as_bytes())
                .find(|&(ref k, _)| k == "url")
                .map(|(_, v)| v)
                .and_then(|url| Uri::from_str(&url).ok())
        });

        let s = format!("{:?}", source_url);
        println!("{}", s);

        source_url.map_or_else(
            || {
                Box::new(futures::future::ok(
                    Response::new().with_status(hyper::BadRequest),
                )) as Box<Future<Item = Self::Response, Error = Self::Error>>
            },
            |source_url| self.get_url(source_url),
        )
    }
}

fn main() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client_handle = core.handle();
    let server_handle = core.handle();

    let addr = "127.0.0.1:3000".parse().unwrap();
    let server = Http::new()
        .serve_addr_handle(&addr, &core.handle(), move || {
            Ok(TurboCrab::new(&client_handle))
        })
        .unwrap();

    handle.spawn(
        server
            .for_each(move |conn| {
                server_handle.spawn(
                    conn.map(|_| ())
                        .map_err(|err| println!("srv1 error: {:?}", err)),
                );
                Ok(())
            })
            .map_err(|_| ()),
    );

    core.run(futures::future::empty::<(), ()>()).unwrap();
}
