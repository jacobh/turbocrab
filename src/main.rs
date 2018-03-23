#![feature(conservative_impl_trait)]

extern crate futures;
extern crate hyper;
extern crate url;

use futures::future::Future;

use hyper::header::ContentLength;
use hyper::server::{Http, Request, Response, Service};

use url::Url;

struct TurboCrab;

impl Service for TurboCrab {
    // boilerplate hooking up hyper's server types
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    // The future representing the eventual Response your call will
    // resolve to. This can change to whatever Future you need.
    type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

    fn call(&self, req: Request) -> Self::Future {
        let source_url: Option<Url> = req.query().and_then(|s| {
            url::form_urlencoded::parse(s.as_bytes())
                .find(|&(ref k, _)| k == "url")
                .map(|(_, v)| v)
                .and_then(|url| Url::parse(&url).ok())
        });

        let s = format!("{:?}", source_url);
        println!("{}", s);

        Box::new(futures::future::ok(
            Response::new()
                .with_header(ContentLength(s.len() as u64))
                .with_body(s),
        ))
    }
}

fn main() {
    let addr = "127.0.0.1:3000".parse().unwrap();
    let server = Http::new().bind(&addr, || Ok(TurboCrab)).unwrap();
    server.run().unwrap();
}
