use std::thread;
use evmap;
use crossbeam_channel;
use hyper::Uri;

use cached_response::CachedResponse;

pub struct TurboCache {
    cache_reader: evmap::ReadHandle<Uri, Box<CachedResponse>>,
    cache_sender: crossbeam_channel::Sender<CachedResponse>,
}
impl TurboCache {
    pub fn new() -> TurboCache {
        let (reader, mut writer) = evmap::new();
        let (tx, rx) = crossbeam_channel::unbounded::<CachedResponse>();

        thread::spawn(move || {
            for cached_response in rx.iter() {
                writer.insert(
                    cached_response.url(),
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
    pub fn get(&self, url: &Uri) -> Option<CachedResponse> {
        self.cache_reader.get_and(url, |resps| *(resps[0].clone()))
    }
    pub fn append_async(&self, resp: CachedResponse) {
        self.cache_sender.send(resp).unwrap()
    }
}
