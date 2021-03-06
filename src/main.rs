// extern crate failure;
extern crate bincode;
extern crate crossbeam_channel;
extern crate futures;
extern crate hyper;
extern crate hyper_tls;
extern crate ring;
#[macro_use]
extern crate serde_derive;
extern crate base64;
extern crate sled;
extern crate tokio_core;
extern crate tokio_file_unix;
extern crate tokio_io;
extern crate url;

mod cache;
mod cached_response;

use cache::TurboCache;
use cached_response::CachedResponseBuilder;

use std::fs::File;
use std::io::Write;
use std::str::FromStr;
use std::sync::Arc;

use futures::prelude::*;

use hyper::server::{Http, Request, Response, Service};
use hyper::{Client, Uri};

use tokio_core::reactor::{Core, Handle};

struct TurboCrab {
    handle: Handle,
    client: Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>,
    cache: Arc<TurboCache>,
}
impl TurboCrab {
    fn new(handle: &Handle) -> TurboCrab {
        TurboCrab {
            handle: handle.clone(),
            client: Client::configure()
                .connector(hyper_tls::HttpsConnector::new(8, handle).unwrap())
                .build(handle),
            cache: Arc::new(TurboCache::new()),
        }
    }
    fn get_url(&self, url: Uri) -> Box<Future<Item = Response, Error = hyper::Error> + 'static> {
        let cache = self.cache.clone();
        let handle = self.handle.clone();

        if let Some(resp) = cache.get(&url) {
            return Box::new(futures::future::ok(resp.into_response(&handle)));
        }

        Box::new(self.client.get(url.clone()).and_then(move |resp| {
            let cache_path = cache::url_to_cache_path(&url);

            let resp_builder =
                CachedResponseBuilder::new(url, resp.status()).with_headers(resp.headers());

            let mut f = File::create(cache_path).unwrap();

            resp.body()
                .for_each(move |chunk| {
                    f.write(&chunk.to_vec())
                        .map(|_| ())
                        .map_err(|e| -> hyper::Error { e.into() })
                })
                .map(move |_| {
                    let cached_response = resp_builder.build();
                    cache.append_async(cached_response.clone());
                    cached_response.into_response(&handle)
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
