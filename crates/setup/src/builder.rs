use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use reqwest::blocking::{Client, ClientBuilder};

use crate::event::Event;
use crate::{Callback, Setup};

/// When adding an "extra bible" we can specify manifest_url and optionally a template
/// for book URLs. The template can include {bible_id} and {book} placeholders, e.g.
/// "https://mycdn.example/bibles/{bible_id}/txt/{book}.json"
#[derive(Debug, Clone)]
pub struct ExtraBible {
    pub id: String,
    pub manifest_url: String,
    pub desc_url: String,
    pub book_url_template: Option<String>,
}

/// Builder for Setup
pub struct SetupBuilder {
    client: ClientBuilder,
    cache_path: PathBuf,
    include_originals: bool,
    extra_bibles: Vec<ExtraBible>,
    callbacks: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    manifest_ttl: Duration,
}

const APP_USER_AGENT: &str = concat!("biblion-setup", "/", env!("CARGO_PKG_VERSION"), "(rust)");

impl Default for SetupBuilder {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .user_agent(APP_USER_AGENT),
            cache_path: PathBuf::from(".cache"),
            include_originals: true,
            extra_bibles: Vec::new(),
            callbacks: HashMap::new(),
            manifest_ttl: Duration::from_secs(60 * 60 * 24 * 30),
        }
    }
}

impl SetupBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn client(mut self, cb: impl Fn(ClientBuilder) -> ClientBuilder) -> Self {
        self.client = cb(self.client);
        self
    }

    pub fn cache_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.cache_path = path.into();
        self
    }

    pub fn include_originals(mut self, value: bool) -> Self {
        self.include_originals = value;
        self
    }

    /// Add an extra bible manifest URL with an optional books URL template.
    pub fn add_bible_from_url<S: Into<String>>(
        mut self,
        id: S,
        manifest_url: S,
        desc_url: S,
        book_url_template: Option<S>,
    ) -> Self {
        self.extra_bibles.push(ExtraBible {
            id: id.into(),
            desc_url: desc_url.into(),
            manifest_url: manifest_url.into(),
            book_url_template: book_url_template.map(|s| s.into()),
        });
        self
    }

    pub fn on<E: Event>(mut self, callback: impl Fn(E::Args) + Send + Sync + 'static) -> Self {
        let type_id = TypeId::of::<E>();
        let boxed: Callback<E::Args> = Box::new(callback);
        self.callbacks.insert(type_id, Box::new(boxed));
        self
    }

    pub fn build(self) -> (Client, Setup) {
        let client = self.client.build().expect("valid client");
        (
            client.clone(),
            Setup {
                client,
                cache_path: self.cache_path,
                include_originals: self.include_originals,
                extra_bibles: self.extra_bibles,
                callbacks: Arc::new(self.callbacks),
                manifest_ttl: self.manifest_ttl,
            },
        )
    }
}
