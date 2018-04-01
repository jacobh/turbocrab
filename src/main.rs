#![feature(conservative_impl_trait)]

// extern crate failure;
extern crate futures;
extern crate hyper;
extern crate hyper_tls;
extern crate tokio_core;
extern crate url;

use std::str::FromStr;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use futures::prelude::*;

use hyper::{Client, Uri};
use hyper::server::{Http, Request, Response, Service};

use tokio_core::reactor::Core;

#[derive(Clone)]
struct CachedResponse {
    status: hyper::StatusCode,
    headers: hyper::Headers,
    body: Vec<u8>,
}
impl Into<Response> for CachedResponse {
    fn into(self) -> Response {
        let CachedResponse {
            status,
            headers,
            body,
        } = self;
        Response::new()
            .with_status(status)
            .with_headers(headers)
            .with_body(body)
    }
}

struct TurboCrab {
    client: Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>,
    cache: Arc<RwLock<HashMap<Uri, CachedResponse>>>,
}

impl TurboCrab {
    fn get_url(&self, url: Uri) -> Box<Future<Item = Response, Error = hyper::Error> + 'static> {
        let cache = self.cache.clone();

        if let Some(resp) = cache.read().unwrap().get(&url) {
            return Box::new(futures::future::ok(resp.clone().into()));
        }

        Box::new(self.client.get(url.clone()).and_then(|resp| {
            let status = resp.status();
            let headers = resp.headers().clone();

            resp.body()
                .concat2()
                .map(|chunk| chunk.to_vec())
                .map(move |body: Vec<u8>| {
                    let cached_response = CachedResponse {
                        status: status,
                        headers: headers,
                        body: body,
                    };
                    cache.write().unwrap().insert(url, cached_response.clone());
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
    let server_handle = handle.clone();

    let addr = "127.0.0.1:3000".parse().unwrap();
    let server = Http::new()
        .serve_addr_handle(&addr, &core.handle(), move || {
            Ok(TurboCrab {
                client: Client::configure()
                    .connector(hyper_tls::HttpsConnector::new(8, &client_handle).unwrap())
                    .build(&client_handle),
                cache: Arc::new(RwLock::new(HashMap::new())),
            })
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
