#![feature(conservative_impl_trait)]

// extern crate failure;
extern crate futures;
extern crate hyper;
extern crate hyper_tls;
extern crate tokio_core;
extern crate url;

use std::str::FromStr;

use futures::prelude::*;

use hyper::header::ContentLength;
use hyper::{Client, Uri};
use hyper::server::{Http, Request, Response, Service};

use tokio_core::reactor::Core;

struct TurboCrab {
    client: Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>,
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

        if let Some(source_url) = source_url {
            Box::new(self.client.get(source_url))
        } else {
            Box::new(futures::future::ok(
                Response::new()
                    .with_header(ContentLength(s.len() as u64))
                    .with_body(s),
            ))
        }
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
