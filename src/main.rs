#![feature(conservative_impl_trait)]

// extern crate failure;
extern crate evmap;
extern crate futures;
extern crate hyper;
extern crate hyper_tls;
extern crate tokio_core;
extern crate url;

use std::str::FromStr;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::prelude::*;

use hyper::{Client, Headers, Uri};
use hyper::server::{Http, Request, Response, Service};

use tokio_core::reactor::{Core, Handle};

struct CachedResponseBuilder {
    status: hyper::StatusCode,
    headers: HashMap<String, Vec<Vec<u8>>>,
    body: Vec<u8>,
}
impl CachedResponseBuilder {
    fn new(status: hyper::StatusCode) -> CachedResponseBuilder {
        CachedResponseBuilder {
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
            status: self.status,
            headers: self.headers,
            body: self.body,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct CachedResponse {
    status: hyper::StatusCode,
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
            .with_status(status)
            .with_headers(headers)
            .with_body(body)
    }
}

struct TurboCrab {
    client: Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>,
    cache_reader: Arc<evmap::ReadHandle<Uri, Box<CachedResponse>>>,
    cache_writer: Arc<Mutex<evmap::WriteHandle<Uri, Box<CachedResponse>>>>,
}
impl TurboCrab {
    fn new(client_handle: &Handle) -> TurboCrab {
        let (reader, writer) = evmap::new();
        TurboCrab {
            client: Client::configure()
                .connector(hyper_tls::HttpsConnector::new(8, client_handle).unwrap())
                .build(client_handle),
            cache_reader: Arc::new(reader),
            cache_writer: Arc::new(Mutex::new(writer)),
        }
    }
    fn get_url(&self, url: Uri) -> Box<Future<Item = Response, Error = hyper::Error> + 'static> {
        let cache_reader = self.cache_reader.clone();

        if let Some(resp) = cache_reader.get_and(&url, |resps| resps[0].clone()) {
            return Box::new(futures::future::ok((*resp).into()));
        }

        let cache_writer = self.cache_writer.clone();

        Box::new(self.client.get(url.clone()).and_then(|resp| {
            let resp_builder =
                CachedResponseBuilder::new(resp.status()).with_headers(resp.headers());

            resp.body()
                .concat2()
                .map(|chunk| chunk.to_vec())
                .map(move |body: Vec<u8>| {
                    let cached_response = resp_builder.with_body(body).build();
                    {
                        let mut cache_writer = cache_writer.lock().unwrap();
                        cache_writer.insert(url, Box::new(cached_response.clone()));
                        cache_writer.refresh()
                    }
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
