use std::thread;
use std::sync::Arc;
use bincode;
use crossbeam_channel;
use hyper::Uri;
use sled::{ConfigBuilder, Tree};

use cached_response::CachedResponse;

pub struct TurboCache {
    tree: Arc<Tree>,
    cache_sender: crossbeam_channel::Sender<CachedResponse>,
}
impl TurboCache {
    pub fn new() -> TurboCache {
        let config = ConfigBuilder::new()
            .path("cachedb")
            .use_compression(false)
            .build();

        let tree = Arc::new(Tree::start(config).unwrap());

        let (tx, rx) = crossbeam_channel::unbounded::<CachedResponse>();

        let tree2 = tree.clone();
        thread::spawn(move || {
            for cached_response in rx.iter() {
                tree2
                    .set(
                        cached_response.url().to_string().as_bytes().to_vec(),
                        bincode::serialize(&cached_response).unwrap(),
                    )
                    .unwrap();
            }
        });

        TurboCache {
            tree: tree,
            cache_sender: tx,
        }
    }
    pub fn get(&self, url: &Uri) -> Option<CachedResponse> {
        self.tree
            .get(url.to_string().as_bytes())
            .unwrap()
            .map(|resp_bytes| bincode::deserialize(&resp_bytes).unwrap())
    }
    pub fn append_async(&self, resp: CachedResponse) {
        self.cache_sender.send(resp).unwrap()
    }
}
